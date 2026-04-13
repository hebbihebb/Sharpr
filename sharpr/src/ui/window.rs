use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use std::path::PathBuf;

use crate::config::AppSettings;
use crate::duplicates::phash;
use crate::model::library::RawImageEntry;
use crate::model::{ImageEntry, LibraryManager};
use crate::thumbnails::ThumbnailWorker;
use crate::ui::filmstrip::FilmstripPane;
use crate::ui::ops_indicator::OpsIndicator;
use crate::ui::preferences::build_preferences_window;
use crate::ui::sidebar::SidebarPane;
use crate::ui::tag_browser::TagBrowser;
use crate::ui::viewer::{ViewerPane, ZoomMode};
use crate::upscale::{UpscaleDetector, UpscaleModel};

// ---------------------------------------------------------------------------
// Shared application state (main thread only, Rc<RefCell<>>)
// ---------------------------------------------------------------------------

pub struct AppState {
    pub library: LibraryManager,
    pub settings: AppSettings,
    pub tags: Option<Arc<crate::tags::TagDatabase>>,
    /// Cached path to the active Vulkan upscaler binary after successful detection.
    pub upscale_binary: Option<PathBuf>,
    pub ops: crate::ops::queue::OpQueue,
}

#[derive(Clone)]
struct PrefetchRequest {
    path: PathBuf,
    index: u32,
    direction: i32,
    distance: u32,
}

struct PrefetchResult {
    request: PrefetchRequest,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

struct ThumbnailOpState {
    total: u32,
    received: u32,
    handle: crate::ops::queue::OpHandle,
}

fn trigger_prefetch(state: &Rc<RefCell<AppState>>, index: u32) {
    let count = state.borrow().library.image_count();
    // Create a channel to receive decoded bytes back on the main thread.
    let (tx, rx) = async_channel::unbounded::<PrefetchResult>();

    for delta in [-1i32, 1i32] {
        queue_prefetch(state, &tx, count, index as i32 + delta, delta, 1);
    }

    // Drain results onto the main thread (channel closes when both senders drop).
    let state_drain = state.clone();
    let tx_drain = tx.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(result) = rx.recv().await {
            let PrefetchResult {
                request,
                bytes,
                width,
                height,
            } = result;
            {
                state_drain.borrow_mut().library.insert_prefetch(
                    request.path.clone(),
                    bytes,
                    width,
                    height,
                );
            }

            if request.distance < 3 {
                let next_index = request.index as i32 + request.direction;
                queue_prefetch(
                    &state_drain,
                    &tx_drain,
                    count,
                    next_index,
                    request.direction,
                    request.distance + 1,
                );
            }
        }
    });
}

fn queue_prefetch(
    state: &Rc<RefCell<AppState>>,
    tx: &async_channel::Sender<PrefetchResult>,
    count: u32,
    index: i32,
    direction: i32,
    distance: u32,
) {
    if index < 0 || index >= count as i32 {
        return;
    }

    let path = {
        let state_ref = state.borrow();
        state_ref
            .library
            .entry_at(index as u32)
            .map(|e: ImageEntry| e.path())
    };
    let Some(path) = path else { return };

    {
        let mut state_ref = state.borrow_mut();
        if state_ref.library.prefetch_pending(&path) {
            return;
        }
        state_ref.library.mark_prefetch_in_flight(path.clone());
    }

    let request = PrefetchRequest {
        path,
        index: index as u32,
        direction,
        distance,
    };
    let tx = tx.clone();
    std::thread::spawn(move || {
        if let Some((bytes, width, height)) = prefetch_decode(&request.path) {
            let _ = tx.send_blocking(PrefetchResult {
                request,
                bytes,
                width,
                height,
            });
        }
    });
}

fn prefetch_decode(path: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}

impl AppState {
    fn new(ops: crate::ops::queue::OpQueue) -> Self {
        let settings = AppSettings::load();
        let mut library = LibraryManager::new();
        library.set_thumbnail_cache_max(settings.thumbnail_cache_max as usize);
        let upscale_binary = settings.upscaler_binary_path.clone();
        Self {
            library,
            settings,
            tags: crate::tags::TagDatabase::open().ok().map(Arc::new),
            upscale_binary,
            ops,
        }
    }
}

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;
    use crate::thumbnails::worker::{HashResult, ThumbnailResult};
    use async_channel::Receiver;

    pub struct SharprWindow {
        pub state: Rc<RefCell<AppState>>,
        pub thumbnail_worker: RefCell<Option<ThumbnailWorker>>,
        // Cloned receiver so the async task can hold it.
        pub result_rx: RefCell<Option<Receiver<ThumbnailResult>>>,
        pub hash_result_rx: RefCell<Option<Receiver<HashResult>>>,
        pub(super) thumbnail_ops: RefCell<HashMap<PathBuf, ThumbnailOpState>>,
    }

    impl Default for SharprWindow {
        fn default() -> Self {
            let (ops_queue, _ops_rx) = crate::ops::queue::new_queue();
            Self {
                state: Rc::new(RefCell::new(AppState::new(ops_queue))),
                thumbnail_worker: RefCell::new(None),
                result_rx: RefCell::new(None),
                hash_result_rx: RefCell::new(None),
                thumbnail_ops: RefCell::new(HashMap::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SharprWindow {
        const NAME: &'static str = "SharprWindow";
        type Type = super::SharprWindow;
        type ParentType = libadwaita::ApplicationWindow;
    }

    impl ObjectImpl for SharprWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for SharprWindow {}
    impl WindowImpl for SharprWindow {}
    impl ApplicationWindowImpl for SharprWindow {}
    impl AdwApplicationWindowImpl for SharprWindow {}
}

glib::wrapper! {
    pub struct SharprWindow(ObjectSubclass<imp::SharprWindow>)
        @extends libadwaita::ApplicationWindow,
                 gtk4::ApplicationWindow,
                 gtk4::Window,
                 gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl SharprWindow {
    pub fn new(app: &libadwaita::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    fn setup(&self) {
        self.set_title(Some("Image Library"));
        self.set_default_size(1400, 900);

        // -----------------------------------------------------------------------
        // Thumbnail worker pool
        // -----------------------------------------------------------------------
        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        // Double the thread count for I/O bound thumbnail loading, cap at 16.
        let thread_count = (cpu_count * 2).clamp(4, 16);
        let (worker, result_rx, hash_result_rx) = ThumbnailWorker::spawn(thread_count);
        *self.imp().thumbnail_worker.borrow_mut() = Some(worker);
        *self.imp().result_rx.borrow_mut() = Some(result_rx);
        *self.imp().hash_result_rx.borrow_mut() = Some(hash_result_rx);

        let state = self.imp().state.clone();
        let (ops_queue, ops_rx) = crate::ops::queue::new_queue();
        state.borrow_mut().ops = ops_queue;

        // -----------------------------------------------------------------------
        // Build panes
        // -----------------------------------------------------------------------
        let sidebar = SidebarPane::new(state.clone());
        let filmstrip = FilmstripPane::new(state.clone());
        let viewer = ViewerPane::new(state.clone());
        viewer.set_metadata_visible(state.borrow().settings.metadata_visible);

        if let Some(worker) = self.imp().thumbnail_worker.borrow().as_ref() {
            filmstrip.set_thumbnail_sender(
                worker.visible_sender(),
                worker.preload_sender(),
                worker.generation_arc(),
                worker.pending_set(),
            );
        }

        let suppress_search_restore = Rc::new(Cell::new(false));
        let content_stack = gtk4::Stack::new();
        let toast_overlay = libadwaita::ToastOverlay::new();
        let tag_browser = state.borrow().tags.clone().map(TagBrowser::new);
        let outer_split = libadwaita::OverlaySplitView::new();
        outer_split.set_max_sidebar_width(280.0);
        outer_split.set_min_sidebar_width(200.0);

        let open_folder: Rc<dyn Fn(PathBuf)> = {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let window_weak = self.downgrade();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let content_stack = content_stack.clone();
            Rc::new(move |path: PathBuf| {
                content_stack.set_visible_child_name("viewer");
                let cache_max = AppSettings::load().thumbnail_cache_max as usize;
                state_c
                    .borrow_mut()
                    .library
                    .set_thumbnail_cache_max(cache_max);
                state_c.borrow_mut().library.reset_for_folder(&path);
                {
                    let mut st = state_c.borrow_mut();
                    st.settings.last_folder = Some(path.clone());
                    st.settings.save();
                }

                let (tx, rx) = async_channel::unbounded::<Vec<RawImageEntry>>();
                let scan_path = path.clone();
                std::thread::spawn(move || {
                    let entries = LibraryManager::scan_folder_raw(&scan_path);
                    let _ = tx.send_blocking(entries);
                });

                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let state_rx = state_c.clone();
                let suppress_search_restore_rx = suppress_search_restore_c.clone();
                let path_rx = path.clone();
                let window_weak_rx = window_weak.clone();
                glib::MainContext::default().spawn_local(async move {
                    let Ok(raw_entries) = rx.recv().await else {
                        return;
                    };

                    {
                        let mut st = state_rx.borrow_mut();
                        if st.library.current_folder.as_deref() != Some(path_rx.as_path()) {
                            return;
                        }

                        let mut new_entries: Vec<ImageEntry> = Vec::with_capacity(raw_entries.len());
                        for (index, raw) in raw_entries.into_iter().enumerate() {
                            let entry = ImageEntry::new(raw.path.clone());
                            entry.set_file_size(raw.file_size);
                            entry.set_dimensions(raw.width, raw.height);
                            st.library.path_to_index.insert(raw.path.clone(), index as u32);
                            st.library.all_known_paths.insert(raw.path);
                            new_entries.push(entry);
                        }
                        st.library.store.splice(0, 0, &new_entries);
                    }

                    sidebar_rx.select_folder(&path_rx);
                    sidebar_rx.set_search_selected(false);
                    sidebar_rx.set_duplicates_selected(false);
                    sidebar_rx.set_tags_selected(false);
                    sidebar_rx.set_quality_selected(None);

                    viewer_rx.clear();
                    filmstrip_rx.refresh();

                    let thumb_total = state_rx.borrow().library.image_count();
                    let thumb_op = if thumb_total > 0 {
                        Some(
                            state_rx
                                .borrow()
                                .ops
                                .add(format!("Loading thumbnails ({thumb_total})")),
                        )
                    } else {
                        None
                    };

                    if let Some(win) = window_weak_rx.upgrade() {
                        if let Some(handle) = thumb_op {
                            win.imp().thumbnail_ops.borrow_mut().insert(
                                path_rx.clone(),
                                ThumbnailOpState {
                                    total: thumb_total,
                                    received: 0,
                                    handle,
                                },
                            );
                        }
                    }

                    let target_index = {
                        let st = state_rx.borrow();
                        st.library
                            .restore_index_for(&path_rx)
                            .filter(|&idx| st.library.entry_at(idx).is_some())
                            .or_else(|| st.library.entry_at(0).map(|_| 0))
                    };

                    if let Some(index) = target_index {
                        state_rx.borrow_mut().library.selected_index = Some(index);
                        filmstrip_rx.navigate_to(index);
                        let path = state_rx
                            .borrow()
                            .library
                            .entry_at(index)
                            .map(|e: ImageEntry| e.path());
                        if let Some(path) = path {
                            viewer_rx.load_image(path);
                        }
                    }

                    if filmstrip_rx.is_search_active() {
                        suppress_search_restore_rx.set(true);
                        filmstrip_rx.deactivate_search();
                    }
                });
            })
        };

        // Sidebar folder selection → scan library → refresh filmstrip.
        {
            let open_folder_c = open_folder.clone();
            sidebar.connect_folder_selected(move |path| {
                open_folder_c(path);
            });
        }

        // Duplicates row → group detection → load_virtual.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let content_stack = content_stack.clone();
            let toast_overlay_c = toast_overlay.clone();
            sidebar.connect_duplicates_selected(move || {
                content_stack.set_visible_child_name("viewer");
                let hashes = state_c.borrow().library.all_hashes_snapshot();
                if hashes.is_empty() {
                    toast_overlay_c.add_toast(libadwaita::Toast::new(
                        "Browse your library first — hashes are computed as thumbnails load",
                    ));
                    return;
                }

                let op = state_c.borrow().ops.add("Finding duplicates");
                let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
                std::thread::spawn(move || {
                    let paths = phash::group_duplicates(&hashes)
                        .into_iter()
                        .filter(|group| group.len() > 1)
                        .flatten()
                        .collect();
                    let _ = tx.send_blocking(paths);
                });

                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let state_rx = state_c.clone();
                let suppress_search_restore_rx = suppress_search_restore_c.clone();
                glib::MainContext::default().spawn_local(async move {
                    let Ok(paths) = rx.recv().await else {
                        op.fail("Detection failed");
                        return;
                    };

                    if paths.is_empty() {
                        op.fail("No duplicates found");
                        return;
                    }

                    state_rx.borrow_mut().library.load_virtual(&paths);
                    sidebar_rx.set_duplicates_selected(true);
                    sidebar_rx.set_search_selected(false);
                    sidebar_rx.set_tags_selected(false);
                    sidebar_rx.set_quality_selected(None);
                    filmstrip_rx.refresh_virtual();
                    let first = state_rx
                        .borrow()
                        .library
                        .entry_at(0)
                        .map(|e: ImageEntry| e.path());
                    if let Some(p) = first {
                        filmstrip_rx.navigate_to(0);
                        viewer_rx.load_image(p);
                    } else {
                        viewer_rx.clear();
                    }
                    if filmstrip_rx.is_search_active() {
                        suppress_search_restore_rx.set(true);
                        filmstrip_rx.deactivate_search();
                    }
                    op.complete();
                });
            });
        }

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let content_stack = content_stack.clone();
            sidebar.connect_search_activated(move || {
                content_stack.set_visible_child_name("viewer");
                sidebar_c.set_search_selected(true);
                sidebar_c.set_duplicates_selected(false);
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                // Show an empty filmstrip immediately so the user knows to type.
                state_c.borrow_mut().library.load_virtual(&[]);
                viewer_c.clear();
                filmstrip_c.refresh_virtual();
                filmstrip_c.activate_search();
            });
        }

        if let Some(tag_browser) = tag_browser.clone() {
            let sidebar_c = sidebar.clone();
            let content_stack_c = content_stack.clone();
            sidebar.connect_tags_selected(move || {
                content_stack_c.set_visible_child_name("tags");
                tag_browser.refresh();
                sidebar_c.set_tags_selected(true);
                sidebar_c.set_quality_selected(None);
            });
        }

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let sidebar_c = sidebar.clone();
            let state_c = state.clone();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let content_stack = content_stack.clone();
            sidebar.connect_quality_selected(move |class| {
                content_stack.set_visible_child_name("viewer");
                let paths: Vec<PathBuf> = {
                    let mut state = state_c.borrow_mut();
                    let settings = state.settings.clone();
                    state.library.paths_for_quality_class(&settings, class)
                };

                state_c.borrow_mut().library.load_virtual(&paths);
                sidebar_c.set_quality_selected(Some(class));
                sidebar_c.set_search_selected(false);
                sidebar_c.set_duplicates_selected(false);
                sidebar_c.set_tags_selected(false);
                viewer_c.clear();
                filmstrip_c.refresh_virtual();
                let first = state_c
                    .borrow()
                    .library
                    .entry_at(0)
                    .map(|e: ImageEntry| e.path());
                if let Some(path) = first {
                    state_c.borrow_mut().library.selected_index = Some(0);
                    filmstrip_c.navigate_to(0);
                    viewer_c.load_image(path);
                }
                if filmstrip_c.is_search_active() {
                    suppress_search_restore_c.set(true);
                    filmstrip_c.deactivate_search();
                }
            });
        }

        // Double-click on filmstrip item → open in default viewer.
        {
            let state_c = state.clone();
            filmstrip.connect_item_activated(move |position| {
                let path = state_c
                    .borrow()
                    .library
                    .entry_at(position)
                    .map(|e: ImageEntry| e.path());
                if let Some(path) = path {
                    let uri = gio::File::for_path(&path).uri();
                    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
                }
            });
        }

        // Filmstrip selection → viewer load.
        {
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            filmstrip.connect_image_selected(move |index| {
                let path = {
                    let Ok(mut state) = state_c.try_borrow_mut() else {
                        return;
                    };
                    state.library.selected_index = Some(index);
                    state
                        .library
                        .entry_at(index)
                        .map(|entry: ImageEntry| entry.path())
                };
                if let Some(path) = path {
                    viewer_c.load_image(path);
                }

                // Queue prefetch for the images immediately before and after this one.
                trigger_prefetch(&state_c, index);
            });
        }

        // Start draining thumbnail results.
        self.start_thumbnail_poll(state.clone(), filmstrip.clone());
        self.start_hash_poll(state.clone());

        // -----------------------------------------------------------------------
        // Layout: AdwOverlaySplitView (outer) → AdwOverlaySplitView (inner)
        // -----------------------------------------------------------------------

        let menu = Self::build_primary_menu();
        let viewer_menu_btn = Self::make_menu_button(&menu);
        viewer_menu_btn.set_visible(true);

        // Inner split: filmstrip sidebar | viewer content.
        let inner_split = libadwaita::OverlaySplitView::new();
        inner_split.set_sidebar_position(gtk4::PackType::Start);

        let filmstrip_page = libadwaita::NavigationPage::builder()
            .title("Photos")
            .tag("filmstrip")
            .child(&filmstrip)
            .build();
        inner_split.set_sidebar(Some(&filmstrip_page));

        let (
            viewer_header,
            sidebar_toggle,
            preview_title_btn,
            commit_btn,
            discard_btn,
            edit_commit_btn,
            edit_discard_btn,
        ) =
            self.build_viewer_header(&viewer_menu_btn);

        let upscale_banner = libadwaita::Banner::new("");
        upscale_banner.set_button_label(Some("Dismiss"));
        upscale_banner.set_revealed(false);
        upscale_banner.connect_button_clicked(|banner| {
            banner.set_revealed(false);
        });

        self.setup_actions(&viewer, state.clone(), &upscale_banner);

        let viewer_toolbar = libadwaita::ToolbarView::new();
        viewer_toolbar.add_top_bar(&viewer_header);
        viewer_toolbar.add_top_bar(&upscale_banner);

        // content_stack lives inside viewer_toolbar so the header bar (hamburger,
        // window controls) is always visible regardless of which page is shown.
        content_stack.add_named(&viewer, Some("viewer"));

        if let Some(tag_browser) = tag_browser.as_ref() {
            content_stack.add_named(tag_browser, Some("tags"));
        }
        content_stack.set_visible_child_name("viewer");

        viewer_toolbar.set_content(Some(&content_stack));
        viewer_toolbar.set_top_bar_style(libadwaita::ToolbarStyle::Raised);

        // Give the viewer a reference to the Commit/Discard buttons so the
        // async upscale callback can show/hide them without extra clones.
        viewer.set_comparison_buttons(commit_btn.clone(), discard_btn.clone());
        viewer.set_edit_buttons(edit_commit_btn.clone(), edit_discard_btn.clone());

        // Commit / Discard buttons for the comparison view.
        {
            let viewer_c = viewer.clone();
            preview_title_btn.connect_clicked(move |_| {
                viewer_c.toggle_debug_comparison();
            });
        }
        {
            let viewer_c = viewer.clone();
            commit_btn.connect_clicked(move |_| {
                viewer_c.commit_upscale();
            });
        }
        {
            let viewer_c = viewer.clone();
            discard_btn.connect_clicked(move |_| {
                viewer_c.discard_upscale();
            });
        }
        {
            let viewer_c = viewer.clone();
            edit_commit_btn.connect_clicked(move |_| {
                viewer_c.save_edit();
            });
        }
        {
            let viewer_c = viewer.clone();
            edit_discard_btn.connect_clicked(move |_| {
                viewer_c.discard_edit();
            });
        }

        let viewer_page = libadwaita::NavigationPage::builder()
            .title("Preview")
            .tag("viewer")
            .child(&viewer_toolbar)
            .build();
        inner_split.set_content(Some(&viewer_page));

        // Update the NavigationPage title to reflect which panel is active.
        {
            let viewer_page_c = viewer_page.clone();
            let preview_title_btn_c = preview_title_btn.clone();
            content_stack.connect_visible_child_notify(move |stack| {
                let name = stack.visible_child_name().unwrap_or_default();
                let is_tags = name == "tags";
                viewer_page_c.set_title(if is_tags { "Tags" } else { "Preview" });
                preview_title_btn_c.set_label(if is_tags { "Tags" } else { "Preview" });
                preview_title_btn_c.set_sensitive(!is_tags);
            });
        }

        // Outer split: explorer sidebar | inner_split.
        let sidebar_overlay = gtk4::Overlay::new();
        sidebar_overlay.set_child(Some(&sidebar));

        let ops_indicator = OpsIndicator::new();
        ops_indicator.set_halign(gtk4::Align::Fill);
        ops_indicator.set_valign(gtk4::Align::End);
        ops_indicator.set_margin_start(12);
        ops_indicator.set_margin_end(12);
        ops_indicator.set_margin_bottom(16);
        sidebar_overlay.add_overlay(&ops_indicator);

        let sidebar_page = libadwaita::NavigationPage::builder()
            .title("Library")
            .tag("sidebar")
            .child(&sidebar_overlay)
            .build();
        outer_split.set_sidebar(Some(&sidebar_page));

        let content_page = libadwaita::NavigationPage::builder()
            .title("Image Library")
            .tag("content")
            .child(&inner_split)
            .build();
        outer_split.set_content(Some(&content_page));

        sidebar_toggle.set_active(true);
        sidebar_toggle
            .bind_property("active", &outer_split, "show-sidebar")
            .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
            .build();

        // -----------------------------------------------------------------------
        // Adaptive breakpoints
        // -----------------------------------------------------------------------

        // < 1200px: collapse explorer sidebar.
        let bp_sidebar = libadwaita::Breakpoint::new(
            libadwaita::BreakpointCondition::parse("max-width: 1200px").unwrap(),
        );
        bp_sidebar.add_setter(&outer_split, "collapsed", Some(&true.to_value()));
        bp_sidebar.add_setter(&outer_split, "show-sidebar", Some(&false.to_value()));
        self.add_breakpoint(bp_sidebar);

        // < 800px: collapse filmstrip into overlay.
        let bp_filmstrip = libadwaita::Breakpoint::new(
            libadwaita::BreakpointCondition::parse("max-width: 800px").unwrap(),
        );
        bp_filmstrip.add_setter(&inner_split, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(bp_filmstrip);

        toast_overlay.set_child(Some(&outer_split));

        self.set_content(Some(&toast_overlay));

        // Drive the indicator from the ops event channel.
        {
            let indicator = ops_indicator.clone();
            glib::MainContext::default().spawn_local(async move {
                use crate::ops::queue::OpEvent;
                while let Ok(event) = ops_rx.recv().await {
                    match event {
                        OpEvent::Added { id, title } => indicator.push_op(id, &title),
                        OpEvent::Progress { id, fraction } => {
                            indicator.update_op(id, fraction)
                        }
                        OpEvent::Completed(id) => indicator.complete_op(id),
                        OpEvent::Failed { id, msg } => indicator.fail_op(id, &msg),
                        OpEvent::Dismissed(id) => indicator.remove_op(id),
                    }
                }
            });
        }

        let builder = gtk4::Builder::from_resource("/io/github/hebbihebb/Sharpr/help-overlay.ui");
        let help_overlay: gtk4::ShortcutsWindow = builder.object("help_overlay").unwrap();
        self.set_help_overlay(Some(&help_overlay));
        filmstrip.set_search_capture_widget(self.upcast_ref::<gtk4::Widget>());

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let content_stack = content_stack.clone();
            let pending_search = Rc::new(Cell::new(None::<glib::SourceId>));
            filmstrip.connect_search_changed(move |text| {
                content_stack.set_visible_child_name("viewer");
                sidebar_c.set_search_selected(!text.trim().is_empty());
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                filmstrip_c.show_autocomplete(vec![]);

                if let Some(source_id) = pending_search.take() {
                    source_id.remove();
                }

                let text = text.trim().to_string();
                if text.is_empty() {
                    state_c.borrow_mut().library.load_virtual(&[]);
                    viewer_c.clear();
                    filmstrip_c.refresh_virtual();
                    return;
                }

                if text.len() < 2 {
                    return;
                }

                let filmstrip_timeout = filmstrip_c.clone();
                let viewer_timeout = viewer_c.clone();
                let state_timeout = state_c.clone();
                let sidebar_timeout = sidebar_c.clone();
                let pending_search_timeout = pending_search.clone();
                let content_stack_timeout = content_stack.clone();
                let source_id =
                    glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
                        pending_search_timeout.set(None);

                        let Some(tags) = state_timeout.borrow().tags.clone() else {
                            return glib::ControlFlow::Break;
                        };

                        let query = text.clone();
                        let db_query = query.clone();
                        let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
                        std::thread::spawn(move || {
                            let _ = tx.send_blocking(tags.search_paths(&db_query));
                        });

                        let filmstrip_rx = filmstrip_timeout.clone();
                        let viewer_rx = viewer_timeout.clone();
                        let state_rx = state_timeout.clone();
                        let sidebar_rx = sidebar_timeout.clone();
                        let content_stack_rx = content_stack_timeout.clone();
                        glib::MainContext::default().spawn_local(async move {
                            if let Ok(db_paths) = rx.recv().await {
                                let q = query.to_lowercase();
                                let library_paths: Vec<PathBuf> = {
                                    let state = state_rx.borrow();
                                    (0..state.library.image_count())
                                        .filter_map(|i| state.library.entry_at(i))
                                        .map(|e| e.path())
                                        .filter(|p| {
                                            p.file_name()
                                                .and_then(|n| n.to_str())
                                                .map(|n| n.to_lowercase().contains(&q))
                                                .unwrap_or(false)
                                        })
                                        .collect()
                                };

                                let mut seen = std::collections::HashSet::new();
                                let merged: Vec<PathBuf> = db_paths
                                    .iter()
                                    .chain(library_paths.iter())
                                    .filter(|p| seen.insert((*p).clone()))
                                    .cloned()
                                    .collect();

                                state_rx.borrow_mut().library.load_virtual(&merged);
                                content_stack_rx.set_visible_child_name("viewer");
                                sidebar_rx.set_search_selected(true);
                                sidebar_rx.set_duplicates_selected(false);
                                sidebar_rx.set_tags_selected(false);
                                sidebar_rx.set_quality_selected(None);
                                viewer_rx.clear();
                                filmstrip_rx.refresh_virtual();
                                let first_path = state_rx
                                    .borrow()
                                    .library
                                    .entry_at(0)
                                    .map(|e: ImageEntry| e.path());
                                if let Some(path) = first_path {
                                    state_rx.borrow_mut().library.selected_index = Some(0);
                                    filmstrip_rx.navigate_to(0);
                                    viewer_rx.load_image(path);
                                }
                            }
                        });

                        glib::ControlFlow::Break
                    });
                pending_search.set(Some(source_id));
            });
        }

        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let content_stack = content_stack.clone();
            filmstrip.connect_search_activate(move |tag| {
                let tag = tag.trim();
                if tag.is_empty() {
                    return;
                }
                let Some(tags) = state_c.borrow().tags.clone() else {
                    return;
                };
                let tag = tag.to_string();
                let (tx, rx) = async_channel::bounded::<Vec<PathBuf>>(1);
                std::thread::spawn(move || {
                    let _ = tx.send_blocking(tags.search_paths(&tag));
                });
                let filmstrip_rx = filmstrip_c.clone();
                let viewer_rx = viewer_c.clone();
                let state_rx = state_c.clone();
                let sidebar_rx = sidebar_c.clone();
                let content_stack_rx = content_stack.clone();
                glib::MainContext::default().spawn_local(async move {
                    if let Ok(paths) = rx.recv().await {
                        state_rx.borrow_mut().library.load_virtual(&paths);
                        content_stack_rx.set_visible_child_name("viewer");
                        sidebar_rx.set_search_selected(true);
                        sidebar_rx.set_duplicates_selected(false);
                        sidebar_rx.set_tags_selected(false);
                        sidebar_rx.set_quality_selected(None);
                        viewer_rx.clear();
                        filmstrip_rx.refresh_virtual();
                        let first_path = state_rx
                            .borrow()
                            .library
                            .entry_at(0)
                            .map(|e: ImageEntry| e.path());
                        if let Some(path) = first_path {
                            state_rx.borrow_mut().library.selected_index = Some(0);
                            filmstrip_rx.navigate_to(0);
                            viewer_rx.load_image(path);
                        }
                    }
                });
            });
        }

        {
            let state_c = state.clone();
            let sidebar_c = sidebar.clone();
            let suppress_search_restore_c = suppress_search_restore.clone();
            let open_folder_c = open_folder.clone();
            filmstrip.connect_search_dismissed(move || {
                if suppress_search_restore_c.replace(false) {
                    return;
                }
                sidebar_c.set_search_selected(false);
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                if state_c.borrow().library.current_folder.is_none() {
                    // Extract last_folder in a separate let so the Ref<AppState> temporary
                    // is dropped at the semicolon — not held alive through the if let body
                    // where open_folder_c calls borrow_mut() and would panic.
                    let last_folder = state_c.borrow().settings.last_folder.clone();
                    if let Some(folder) = last_folder {
                        if folder.is_dir() {
                            open_folder_c(folder);
                        }
                    }
                }
            });
        }

        if let Some(tag_browser) = tag_browser {
            let content_stack_c = content_stack.clone();
            let sidebar_c = sidebar.clone();
            let filmstrip_c = filmstrip.clone();
            tag_browser.connect_tag_activated(move |tag| {
                content_stack_c.set_visible_child_name("viewer");
                sidebar_c.set_tags_selected(false);
                sidebar_c.set_quality_selected(None);
                filmstrip_c.emit_search_activate(tag);
            });
        }

        // -----------------------------------------------------------------------
        // Alt+Left / Alt+Right — navigate between images.
        // Scoped to the window so it fires regardless of focus position.
        // -----------------------------------------------------------------------
        self.setup_nav_shortcuts(
            state.clone(),
            filmstrip.clone(),
            viewer.clone(),
            outer_split.clone(),
        );

        // -----------------------------------------------------------------------
        // Restore last folder
        // -----------------------------------------------------------------------
        let last_folder = state.borrow().settings.last_folder.clone();
        let start_folder = last_folder
            .filter(|p| p.is_dir())
            .or_else(|| sidebar.first_folder_path());

        // Defer by one idle cycle so widgets are realized before we call
        // open_folder(). Without this, smart_list.unselect_all() fires before
        // realization and the Duplicates row stays visually highlighted.
        if let Some(folder) = start_folder {
            glib::idle_add_local_once(move || {
                open_folder(folder);
            });
        }
    }

    /// Wire Alt+Left / Alt+Right to advance the image selection.
    fn setup_nav_shortcuts(
        &self,
        state: Rc<RefCell<AppState>>,
        filmstrip: FilmstripPane,
        viewer: ViewerPane,
        outer_split: libadwaita::OverlaySplitView,
    ) {
        let shortcuts = gtk4::ShortcutController::new();
        // Managed scope means the window itself handles these even when a child
        // widget has focus.
        shortcuts.set_scope(gtk4::ShortcutScope::Managed);

        let make_action = |state: Rc<RefCell<AppState>>,
                           filmstrip: FilmstripPane,
                           viewer: ViewerPane,
                           delta: i32| {
            gtk4::CallbackAction::new(move |_, _| {
                let new_index = {
                    let Ok(mut st) = state.try_borrow_mut() else {
                        return glib::Propagation::Proceed;
                    };
                    st.library.navigate(delta)
                };
                if let Some(index) = new_index {
                    filmstrip.navigate_to(index);
                    let path = state
                        .borrow()
                        .library
                        .entry_at(index)
                        .map(|e: ImageEntry| e.path());
                    if let Some(p) = path {
                        viewer.load_image(p);
                    }
                }
                glib::Propagation::Stop
            })
        };

        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Alt>Left").unwrap()),
            Some(make_action(
                state.clone(),
                filmstrip.clone(),
                viewer.clone(),
                -1,
            )),
        ));
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Alt>Right").unwrap()),
            Some(make_action(
                state.clone(),
                filmstrip.clone(),
                viewer.clone(),
                1,
            )),
        ));

        // F11 — Toggle fullscreen.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("F11").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                if let Some(win) = widget.downcast_ref::<gtk4::Window>() {
                    if win.is_fullscreen() {
                        win.unfullscreen();
                    } else {
                        win.fullscreen();
                    }
                }
                glib::Propagation::Stop
            })),
        ));

        // F9 — Toggle Library sidebar.
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("F9").unwrap()),
            Some(gtk4::CallbackAction::new(move |_, _| {
                outer_split.set_show_sidebar(!outer_split.shows_sidebar());
                glib::Propagation::Stop
            })),
        ));

        // Del — Move selected image to trash.
        {
            let state_d = state.clone();
            let filmstrip_d = filmstrip.clone();
            let viewer_d = viewer.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("Delete").unwrap()),
                Some(gtk4::CallbackAction::new(move |_, _| {
                    let (path, index) = {
                        let Ok(st) = state_d.try_borrow() else {
                            return glib::Propagation::Proceed;
                        };
                        let index = match st.library.selected_index {
                            Some(i) => i,
                            None => return glib::Propagation::Proceed,
                        };
                        let path = match st.library.entry_at(index) {
                            Some(e) => e.path(),
                            None => return glib::Propagation::Proceed,
                        };
                        (path, index)
                    };

                    if gio::File::for_path(&path)
                        .trash(None::<&gio::Cancellable>)
                        .is_ok()
                    {
                        if let Some(tags) = state_d.borrow().tags.clone() {
                            tags.remove_path(&path);
                        }
                        state_d.borrow_mut().library.remove_path(&path);
                        let new_count = state_d.borrow().library.image_count();
                        if new_count == 0 {
                            viewer_d.clear();
                        } else {
                            let new_index = index.min(new_count - 1);
                            filmstrip_d.navigate_to(new_index);
                            let next_path = state_d
                                .borrow()
                                .library
                                .entry_at(new_index)
                                .map(|e: ImageEntry| e.path());
                            if let Some(p) = next_path {
                                viewer_d.load_image(p);
                            }
                        }
                    }
                    glib::Propagation::Stop
                })),
            ));
        }

        // Context-menu "Move to Trash" from filmstrip (duplicates mode).
        {
            let state_tr = state.clone();
            let filmstrip_tr = filmstrip.clone();
            let viewer_tr = viewer.clone();
            filmstrip.connect_trash_requested(move |path| {
                if gio::File::for_path(&path)
                    .trash(None::<&gio::Cancellable>)
                    .is_ok()
                {
                    if let Some(tags) = state_tr.borrow().tags.clone() {
                        tags.remove_path(&path);
                    }
                    state_tr.borrow_mut().library.remove_path(&path);
                    let new_count = state_tr.borrow().library.image_count();
                    if new_count == 0 {
                        viewer_tr.clear();
                    } else {
                        let index = state_tr.borrow().library.selected_index.unwrap_or(0);
                        let new_index = index.min(new_count - 1);
                        filmstrip_tr.navigate_to(new_index);
                        let next_path = state_tr
                            .borrow()
                            .library
                            .entry_at(new_index)
                            .map(|e: ImageEntry| e.path());
                        if let Some(p) = next_path {
                            viewer_tr.load_image(p);
                        }
                    }
                }
            });
        }

        // Ctrl+T — open the tag editor for the selected image.
        {
            let state_t = state.clone();
            let viewer_t = viewer.clone();
            shortcuts.add_shortcut(gtk4::Shortcut::new(
                Some(gtk4::ShortcutTrigger::parse_string("<Control>T").unwrap()),
                Some(gtk4::CallbackAction::new(move |widget, _| {
                    if let Some(window) = widget.downcast_ref::<gtk4::Window>() {
                        if let Some(focus) = gtk4::prelude::GtkWindowExt::focus(window) {
                            if focus.is::<gtk4::Text>() || focus.is::<gtk4::SearchEntry>() {
                                return glib::Propagation::Proceed;
                            }
                        }
                    }

                    let has_selection = state_t
                        .try_borrow()
                        .ok()
                        .and_then(|state| state.library.selected_index)
                        .is_some();
                    if !has_selection {
                        return glib::Propagation::Stop;
                    }

                    viewer_t.open_tag_popover();
                    glib::Propagation::Stop
                })),
            ));
        }

        self.add_controller(shortcuts);
    }

    /// Drain thumbnail results from the worker pool on the GLib main context.
    /// For each result: construct a `MemoryTexture` on the main thread,
    /// find the matching `ImageEntry` in the store, and update it.
    fn start_thumbnail_poll(&self, state: Rc<RefCell<AppState>>, filmstrip: FilmstripPane) {
        let result_rx = self.imp().result_rx.borrow().clone();
        let Some(rx) = result_rx else { return };
        let window_weak = self.downgrade();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                use gdk4::{MemoryFormat, MemoryTexture};
                use glib::Bytes;

                let result_path = result.path.clone();
                let bytes = Bytes::from_owned(result.rgba_bytes);
                let texture = MemoryTexture::new(
                    result.width as i32,
                    result.height as i32,
                    MemoryFormat::R8g8b8a8,
                    &bytes,
                    (result.width * 4) as usize,
                );

                // Use the path→index lookup built by LibraryManager to avoid a
                // linear scan for every completed thumbnail.
                {
                    let st = state.borrow();
                    if let Some(idx) = st.library.index_of_path(&result_path) {
                        if let Some(entry) = st.library.entry_at(idx) {
                            entry.set_thumbnail(Some(texture.clone().upcast::<gdk4::Texture>()));
                        }
                    }
                }

                // Cache the texture in LibraryManager (needs mut borrow).
                {
                    state
                        .borrow_mut()
                        .library
                        .insert_thumbnail(result_path.clone(), texture.upcast());
                }

                filmstrip.mark_thumbnail_ready(&result_path);

                if let Some(window) = window_weak.upgrade() {
                    if let Some(folder) = result_path.parent().map(|p| p.to_path_buf()) {
                        let mut thumbnail_ops = window.imp().thumbnail_ops.borrow_mut();
                        if let Some(op_state) = thumbnail_ops.get_mut(&folder) {
                            op_state.received += 1;
                            op_state.handle.progress(Some(
                                op_state.received as f32 / op_state.total.max(1) as f32,
                            ));
                            if op_state.total > 0 && op_state.received >= op_state.total {
                                if let Some(completed) = thumbnail_ops.remove(&folder) {
                                    completed.handle.complete();
                                }
                            }
                        }
                    }
                }
            }

            if let Some(window) = window_weak.upgrade() {
                for (_, op_state) in window.imp().thumbnail_ops.borrow_mut().drain() {
                    op_state.handle.complete();
                }
            }
        });
    }

    fn start_hash_poll(&self, state: Rc<RefCell<AppState>>) {
        let hash_rx = self.imp().hash_result_rx.borrow().clone();
        let Some(rx) = hash_rx else { return };

        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                let path = result.path.clone();
                state
                    .borrow_mut()
                    .library
                    .insert_hash(result.path, result.hash);
                if let Some(tags_arc) = state.borrow().tags.clone() {
                    std::thread::spawn(move || {
                        let meta = crate::metadata::exif::ImageMetadata::load(&path);
                        let tag_list = crate::tags::indexer::index_entry(&path, &meta);
                        tags_arc.insert_tags(&path, &tag_list);
                    });
                }
            }
        });
    }

    fn build_primary_menu() -> gio::Menu {
        let menu = gio::Menu::new();

        let view_section = gio::Menu::new();
        let zoom_subsection = gio::Menu::new();
        zoom_subsection.append(Some("Fit to Window"), Some("win.zoom-mode::fit"));
        zoom_subsection.append(Some("1:1 Pixels"), Some("win.zoom-mode::1:1"));
        view_section.append_section(None, &zoom_subsection);
        view_section.append(Some("Show Metadata"), Some("win.show-metadata"));
        menu.append_section(Some("View"), &view_section);

        let transform_section = gio::Menu::new();
        transform_section.append(Some("Rotate 90° Clockwise"), Some("win.rotate-cw"));
        transform_section.append(Some("Rotate 90° Counter-Clockwise"), Some("win.rotate-ccw"));
        transform_section.append(Some("Flip Horizontal"), Some("win.flip-h"));
        transform_section.append(Some("Flip Vertical"), Some("win.flip-v"));
        menu.append_section(Some("Rotate & Flip"), &transform_section);

        let upscale_section = gio::Menu::new();
        upscale_section.append(Some("Upscale Image…"), Some("win.upscale"));
        menu.append_section(Some("AI Upscale"), &upscale_section);

        let app_section = gio::Menu::new();
        app_section.append(Some("Keyboard Shortcuts"), Some("win.show-help-overlay"));
        app_section.append(Some("Preferences"), Some("win.show-preferences"));
        app_section.append(Some("About Sharpr"), Some("app.about"));
        menu.append_section(Some("App"), &app_section);

        menu
    }

    fn make_menu_button(menu: &gio::Menu) -> gtk4::MenuButton {
        let popover = gtk4::PopoverMenu::from_model(Some(menu));
        let btn = gtk4::MenuButton::new();
        btn.set_icon_name("open-menu-symbolic");
        btn.set_tooltip_text(Some("Main Menu"));
        btn.set_popover(Some(&popover));
        btn.add_css_class("flat");
        btn
    }

    fn setup_actions(
        &self,
        viewer: &ViewerPane,
        state: Rc<RefCell<AppState>>,
        upscale_banner: &libadwaita::Banner,
    ) {
        let zoom_initial = match viewer.zoom_mode() {
            ZoomMode::Fit => "fit",
            ZoomMode::OneToOne => "1:1",
        };
        let zoom_action = gio::SimpleAction::new_stateful(
            "zoom-mode",
            Some(glib::VariantTy::STRING),
            &zoom_initial.to_variant(),
        );
        {
            let viewer_weak = viewer.downgrade();
            zoom_action.connect_activate(move |action, param| {
                let (Some(viewer), Some(param)) = (viewer_weak.upgrade(), param) else {
                    return;
                };
                let mode = if param.str() == Some("1:1") {
                    ZoomMode::OneToOne
                } else {
                    ZoomMode::Fit
                };
                viewer.set_zoom_mode(mode);
                action.set_state(param);
            });
        }
        self.add_action(&zoom_action);

        let meta_action = gio::SimpleAction::new_stateful(
            "show-metadata",
            None,
            &viewer.metadata_visible().to_variant(),
        );
        {
            let viewer_weak = viewer.downgrade();
            meta_action.connect_activate(move |action, _| {
                let Some(viewer) = viewer_weak.upgrade() else {
                    return;
                };
                let new_val = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
                viewer.set_metadata_visible(new_val);
                action.set_state(&new_val.to_variant());
            });
        }
        self.add_action(&meta_action);

        let pref_action = gio::SimpleAction::new("show-preferences", None);
        {
            let window_weak = self.downgrade();
            let state_c = state.clone();
            pref_action.connect_activate(move |_, _| {
                let Some(window) = window_weak.upgrade() else {
                    return;
                };
                let settings = state_c.borrow().settings.clone();
                let prefs = build_preferences_window(&settings, &window);
                prefs.present();
            });
        }
        self.add_action(&pref_action);

        for name in ["rotate-cw", "rotate-ccw", "flip-h", "flip-v"] {
            let a = gio::SimpleAction::new(name, None);
            let viewer_weak = viewer.downgrade();
            let op = name.to_owned();
            a.connect_activate(move |_, _| {
                if let Some(v) = viewer_weak.upgrade() {
                    v.apply_transform(&op);
                }
            });
            self.add_action(&a);
        }

        let upscale_action = gio::SimpleAction::new("upscale", None);
        {
            let state_c = state.clone();
            let banner_c = upscale_banner.clone();
            let viewer_weak = viewer.downgrade();
            let action_weak = upscale_action.downgrade();
            upscale_action.connect_activate(move |_action, _| {
                let Some(viewer) = viewer_weak.upgrade() else { return };
                let has_binary = {
                    let mut st = state_c.borrow_mut();
                    if st.upscale_binary.is_none() {
                        st.upscale_binary = AppSettings::load()
                            .upscaler_binary_path
                            .filter(|path| path.is_file());
                    }
                    if st.upscale_binary.is_none() {
                        st.upscale_binary = UpscaleDetector::find_realesrgan();
                    }
                    st.upscale_binary.is_some()
                };
                if !has_binary {
                    banner_c.set_title(
                        "AI upscaling requires a supported Vulkan backend such as upscayl-bin or realesrgan-ncnn-vulkan.",
                    );
                    banner_c.set_revealed(true);
                    return;
                }
                banner_c.set_revealed(false);
                let path = state_c
                    .borrow()
                    .library
                    .selected_entry()
                    .map(|e: ImageEntry| e.path());
                if let Some(p) = path {
                    let dialog = libadwaita::AlertDialog::new(Some("Upscale Image"), None);
                    dialog.add_response("cancel", "Cancel");
                    dialog.add_response("upscale", "Upscale");
                    dialog.set_default_response(Some("upscale"));
                    dialog.set_close_response("cancel");
                    dialog.set_response_appearance(
                        "upscale",
                        libadwaita::ResponseAppearance::Suggested,
                    );

                    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
                    content.set_margin_top(6);
                    content.set_margin_bottom(6);

                    let model_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let model_label = gtk4::Label::new(Some("Model"));
                    model_label.set_halign(gtk4::Align::Start);
                    model_box.append(&model_label);

                    let saved_model =
                        state_c.borrow().settings.upscaler_default_model.clone();
                    let standard_btn =
                        gtk4::CheckButton::with_label("Standard - best for photos");
                    let anime_btn =
                        gtk4::CheckButton::with_label("Anime / Art - best for illustration");
                    anime_btn.set_group(Some(&standard_btn));
                    if saved_model == "anime" {
                        anime_btn.set_active(true);
                    } else {
                        standard_btn.set_active(true);
                    }
                    model_box.append(&standard_btn);
                    model_box.append(&anime_btn);
                    content.append(&model_box);

                    let scale_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                    let scale_label = gtk4::Label::new(Some("Scale"));
                    scale_label.set_halign(gtk4::Align::Start);
                    scale_box.append(&scale_label);

                    let scale_dropdown =
                        gtk4::DropDown::from_strings(&["Smart (auto)", "2×", "3×", "4×"]);
                    scale_dropdown.set_selected(0);
                    scale_box.append(&scale_dropdown);
                    content.append(&scale_box);

                    dialog.set_extra_child(Some(&content));

                    let viewer_weak_c = viewer_weak.clone();
                    let action_weak_c = action_weak.clone();
                    dialog.choose(
                        &viewer,
                        None::<&gio::Cancellable>,
                        move |response| {
                            if response != "upscale" {
                                return;
                            }
                            let Some(viewer) = viewer_weak_c.upgrade() else {
                                return;
                            };
                            if let Some(action) = action_weak_c.upgrade() {
                                action.set_enabled(false);
                            }
                            // Proxy button bridges start_upscale's re-enable callback back
                            // to the action: start insensitive so the true→false→true
                            // transition fires the notify when the job finishes.
                            let proxy_btn = gtk4::Button::new();
                            let action_weak_c = action_weak_c.clone();
                            proxy_btn.connect_sensitive_notify(move |btn| {
                                if btn.is_sensitive() {
                                    if let Some(a) = action_weak_c.upgrade() {
                                        a.set_enabled(true);
                                    }
                                }
                            });
                            proxy_btn.set_sensitive(false);

                            let model = if anime_btn.is_active() {
                                UpscaleModel::Anime
                            } else {
                                UpscaleModel::Standard
                            };
                            let scale = match scale_dropdown.selected() {
                                1 => 2,
                                2 => 3,
                                3 => 4,
                                _ => 0,
                            };
                            viewer.start_upscale(p.clone(), scale, model, proxy_btn);
                        },
                    );
                }
            });
        }
        self.add_action(&upscale_action);
    }

    /// Build the viewer header bar.
    /// Returns `(header, sidebar_toggle, preview_title_btn, commit_btn, discard_btn, edit_commit_btn, edit_discard_btn)`.
    /// Commit and Discard are initially hidden; the comparison view shows them.
    fn build_viewer_header(
        &self,
        menu_btn: &gtk4::MenuButton,
    ) -> (
        libadwaita::HeaderBar,
        gtk4::ToggleButton,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
    ) {
        let header = libadwaita::HeaderBar::new();

        let sidebar_toggle = gtk4::ToggleButton::new();
        sidebar_toggle.set_icon_name("sidebar-show-symbolic");
        sidebar_toggle.set_tooltip_text(Some("Toggle Library"));
        sidebar_toggle.add_css_class("flat");
        header.pack_start(&sidebar_toggle);

        let preview_title_btn = gtk4::Button::with_label("Preview");
        preview_title_btn.add_css_class("flat");
        preview_title_btn.add_css_class("title");
        preview_title_btn.set_focus_on_click(false);
        preview_title_btn.set_tooltip_text(Some("Temporary debug compare toggle"));
        header.set_title_widget(Some(&preview_title_btn));

        let commit_btn = gtk4::Button::with_label("Save");
        commit_btn.set_tooltip_text(Some("Save upscaled image"));
        commit_btn.add_css_class("suggested-action");
        commit_btn.set_visible(false);
        header.pack_end(&commit_btn);

        let discard_btn = gtk4::Button::with_label("Discard");
        discard_btn.set_tooltip_text(Some("Discard upscaled image"));
        discard_btn.add_css_class("destructive-action");
        discard_btn.set_visible(false);
        header.pack_end(&discard_btn);

        let edit_commit_btn = gtk4::Button::with_label("Save Edit");
        edit_commit_btn.set_tooltip_text(Some("Save the rotated/flipped image to disk"));
        edit_commit_btn.add_css_class("suggested-action");
        edit_commit_btn.set_visible(false);
        header.pack_end(&edit_commit_btn);

        let edit_discard_btn = gtk4::Button::with_label("Discard Edit");
        edit_discard_btn.set_tooltip_text(Some("Revert to the original image"));
        edit_discard_btn.add_css_class("destructive-action");
        edit_discard_btn.set_visible(false);
        header.pack_end(&edit_discard_btn);

        header.pack_end(menu_btn);

        (
            header,
            sidebar_toggle,
            preview_title_btn,
            commit_btn,
            discard_btn,
            edit_commit_btn,
            edit_discard_btn,
        )
    }
}
