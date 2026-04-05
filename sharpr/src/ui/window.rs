use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use std::path::PathBuf;

use crate::config::AppSettings;
use crate::model::{ImageEntry, LibraryManager};
use crate::thumbnails::ThumbnailWorker;
use crate::ui::filmstrip::FilmstripPane;
use crate::ui::sidebar::SidebarPane;
use crate::ui::viewer::ViewerPane;
use crate::upscale::{UpscaleDetector, UpscaleModel};

// ---------------------------------------------------------------------------
// Shared application state (main thread only, Rc<RefCell<>>)
// ---------------------------------------------------------------------------

pub struct AppState {
    pub library: LibraryManager,
    pub settings: AppSettings,
    /// Cached path to the realesrgan-ncnn-vulkan binary after successful detection.
    pub upscale_binary: Option<PathBuf>,
    pub upscale_model: UpscaleModel,
}

impl AppState {
    fn new() -> Self {
        Self {
            library: LibraryManager::new(),
            settings: AppSettings::load(),
            upscale_binary: None,
            upscale_model: UpscaleModel::Standard,
        }
    }
}

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;
    use crate::thumbnails::worker::ThumbnailResult;
    use async_channel::Receiver;

    pub struct SharprWindow {
        pub state: Rc<RefCell<AppState>>,
        pub thumbnail_worker: RefCell<Option<ThumbnailWorker>>,
        // Cloned receiver so the async task can hold it.
        pub result_rx: RefCell<Option<Receiver<ThumbnailResult>>>,
    }

    impl Default for SharprWindow {
        fn default() -> Self {
            Self {
                state: Rc::new(RefCell::new(AppState::new())),
                thumbnail_worker: RefCell::new(None),
                result_rx: RefCell::new(None),
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
        let (worker, result_rx) = ThumbnailWorker::spawn(4);
        *self.imp().thumbnail_worker.borrow_mut() = Some(worker);
        *self.imp().result_rx.borrow_mut() = Some(result_rx);

        let state = self.imp().state.clone();

        // -----------------------------------------------------------------------
        // Build panes
        // -----------------------------------------------------------------------
        let sidebar = SidebarPane::new(state.clone());
        let filmstrip = FilmstripPane::new(state.clone());
        let viewer = ViewerPane::new(state.clone());

        if let Some(worker) = self.imp().thumbnail_worker.borrow().as_ref() {
            filmstrip.set_thumbnail_sender(worker.sender(), worker.generation_arc());
        }

        // Sidebar folder selection → scan library → refresh filmstrip.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            let window_weak = self.downgrade();
            sidebar.connect_folder_selected(move |path| {
                // Bump generation so workers drop stale requests from the
                // previous folder immediately rather than blocking the queue.
                if let Some(win) = window_weak.upgrade() {
                    if let Some(worker) = win.imp().thumbnail_worker.borrow().as_ref() {
                        worker.bump_generation();
                    }
                }
                state_c.borrow_mut().library.scan_folder(&path);
                // Persist last folder.
                state_c.borrow_mut().settings.last_folder = Some(path.clone());
                state_c.borrow().settings.save();

                viewer_c.clear();
                filmstrip_c.refresh();

                // Explicitly load the first image — selected_notify may not
                // fire if the new folder also lands at position 0.
                let first = state_c.borrow().library.entry_at(0).map(|e: ImageEntry| e.path());
                if let Some(p) = first {
                    filmstrip_c.select_index(0);
                    viewer_c.load_image(p);
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
                        // GTK can emit selection notifications re-entrantly while the
                        // library store is being rebuilt; ignore those transient events.
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
            });
        }

        // Start draining thumbnail results.
        self.start_thumbnail_poll(state.clone(), filmstrip.clone());

        // -----------------------------------------------------------------------
        // Layout: AdwNavigationSplitView (outer) → AdwOverlaySplitView (inner)
        // -----------------------------------------------------------------------

        // Inner split: filmstrip sidebar | viewer content.
        let inner_split = libadwaita::OverlaySplitView::new();
        inner_split.set_sidebar_position(gtk4::PackType::Start);

        let filmstrip_page = libadwaita::NavigationPage::builder()
            .title("Photos")
            .tag("filmstrip")
            .child(&filmstrip)
            .build();
        inner_split.set_sidebar(Some(&filmstrip_page));

        let (viewer_header, zoom_btn, upscale_model_dd, upscale_btn, commit_btn, discard_btn) =
            self.build_viewer_header();

        viewer.set_zoom_button(zoom_btn.clone());
        {
            let viewer_c = viewer.clone();
            zoom_btn.connect_clicked(move |_| { viewer_c.toggle_zoom_mode(); });
        }
        let upscale_banner = libadwaita::Banner::new("");
        upscale_banner.set_button_label(Some("Dismiss"));
        upscale_banner.set_revealed(false);
        upscale_banner.connect_button_clicked(|banner| {
            banner.set_revealed(false);
        });

        let viewer_toolbar = libadwaita::ToolbarView::new();
        viewer_toolbar.add_top_bar(&viewer_header);
        viewer_toolbar.add_top_bar(&upscale_banner);

        viewer_toolbar.set_content(Some(&viewer));
        viewer_toolbar.set_top_bar_style(libadwaita::ToolbarStyle::Raised);

        // Upscale button: lazy-detect binary on first click; run job when available.
        {
            let state_c = state.clone();
            upscale_model_dd.connect_selected_notify(move |dropdown| {
                let model = match dropdown.selected() {
                    1 => UpscaleModel::Anime,
                    _ => UpscaleModel::Standard,
                };
                state_c.borrow_mut().upscale_model = model;
            });
        }

        {
            let state_c = state.clone();
            let banner_c = upscale_banner.clone();
            let viewer_c = viewer.clone();
            upscale_btn.connect_clicked(move |btn| {
                // Lazy binary detection — cached after first successful find.
                let has_binary = {
                    let mut st = state_c.borrow_mut();
                    if st.upscale_binary.is_none() {
                        st.upscale_binary = UpscaleDetector::find_realesrgan();
                    }
                    st.upscale_binary.is_some()
                };

                if !has_binary {
                    banner_c.set_title(
                        "AI upscaling requires realesrgan-ncnn-vulkan.                          Install it to ~/.local/bin or via Flatpak.",
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
                    btn.set_sensitive(false);
                    viewer_c.start_upscale(p, btn.clone());
                }
            });
        }

        // Give the viewer a reference to the Commit/Discard buttons so the
        // async upscale callback can show/hide them without extra clones.
        viewer.set_comparison_buttons(commit_btn.clone(), discard_btn.clone());

        // Commit / Discard buttons for the comparison view.
        {
            let viewer_c = viewer.clone();
            commit_btn.connect_clicked(move |_| { viewer_c.commit_upscale(); });
        }
        {
            let viewer_c = viewer.clone();
            discard_btn.connect_clicked(move |_| { viewer_c.discard_upscale(); });
        }

        let viewer_page = libadwaita::NavigationPage::builder()
            .title("Preview")
            .tag("viewer")
            .child(&viewer_toolbar)
            .build();
        inner_split.set_content(Some(&viewer_page));

        // Outer split: explorer sidebar | inner_split.
        let outer_split = libadwaita::NavigationSplitView::new();

        let sidebar_page = libadwaita::NavigationPage::builder()
            .title("Library")
            .tag("sidebar")
            .child(&sidebar)
            .build();
        outer_split.set_sidebar(Some(&sidebar_page));

        let content_page = libadwaita::NavigationPage::builder()
            .title("Image Library")
            .tag("content")
            .child(&inner_split)
            .build();
        outer_split.set_content(Some(&content_page));

        // -----------------------------------------------------------------------
        // Adaptive breakpoints
        // -----------------------------------------------------------------------

        // < 1200px: collapse explorer sidebar.
        let bp_sidebar = libadwaita::Breakpoint::new(
            libadwaita::BreakpointCondition::parse("max-width: 1200px").unwrap(),
        );
        bp_sidebar.add_setter(&outer_split, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(bp_sidebar);

        // < 800px: collapse filmstrip into overlay.
        let bp_filmstrip = libadwaita::Breakpoint::new(
            libadwaita::BreakpointCondition::parse("max-width: 800px").unwrap(),
        );
        bp_filmstrip.add_setter(&inner_split, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(bp_filmstrip);

        self.set_content(Some(&outer_split));

        // -----------------------------------------------------------------------
        // Alt+Left / Alt+Right — navigate between images.
        // Scoped to the window so it fires regardless of focus position.
        // -----------------------------------------------------------------------
        self.setup_nav_shortcuts(state.clone(), filmstrip.clone(), viewer.clone());

        // -----------------------------------------------------------------------
        // Restore last folder
        // -----------------------------------------------------------------------
        let last_folder = state.borrow().settings.last_folder.clone();
        if let Some(folder) = last_folder {
            if folder.is_dir() {
                state.borrow_mut().library.scan_folder(&folder);
                filmstrip.refresh();
                let first = state.borrow().library.entry_at(0).map(|e: ImageEntry| e.path());
                if let Some(p) = first {
                    filmstrip.select_index(0);
                    viewer.load_image(p);
                }
            }
        }
    }

    /// Wire Alt+Left / Alt+Right to advance the image selection.
    fn setup_nav_shortcuts(
        &self,
        state: Rc<RefCell<AppState>>,
        filmstrip: FilmstripPane,
        viewer: ViewerPane,
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
                    filmstrip.select_index(index);
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
            Some(make_action(state.clone(), filmstrip.clone(), viewer.clone(), 1)),
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
                        state_d.borrow_mut().library.remove_path(&path);
                        let new_count = state_d.borrow().library.image_count();
                        if new_count == 0 {
                            viewer_d.clear();
                        } else {
                            let new_index = index.min(new_count - 1);
                            filmstrip_d.select_index(new_index);
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

        self.add_controller(shortcuts);
    }

    /// Drain thumbnail results from the worker pool on the GLib main context.
    /// For each result: construct a `MemoryTexture` on the main thread,
    /// find the matching `ImageEntry` in the store, and update it.
    fn start_thumbnail_poll(&self, state: Rc<RefCell<AppState>>, filmstrip: FilmstripPane) {
        let result_rx = self.imp().result_rx.borrow().clone();
        let Some(rx) = result_rx else { return };

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
                let found = {
                    let st = state.borrow();
                    if let Some(idx) = st.library.index_of_path(&result_path) {
                        if let Some(entry) = st.library.entry_at(idx) {
                            entry.set_thumbnail(texture.clone().upcast());
                            Some(idx)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                // Cache the texture in LibraryManager (needs mut borrow).
                {
                    state
                        .borrow_mut()
                        .library
                        .insert_thumbnail(result_path.clone(), texture.upcast());
                }

                // Notify GtkListView that item `i` changed (triggers re-bind).
                if let Some(idx) = found {
                    state.borrow().library.store.items_changed(idx, 1, 1);
                }

                filmstrip.mark_thumbnail_ready(&result_path);
                filmstrip.schedule_visible_thumbnails();
            }
        });
    }

    /// Build the viewer header bar.
    /// Returns `(header_bar, upscale_button)` so the caller can wire detector
    /// behavior once the rest of the viewer widgets exist.
    /// Returns `(header, zoom_btn, model_dropdown, upscale_btn, commit_btn, discard_btn)`.
    /// Commit and Discard are initially hidden; the comparison view shows them.
    fn build_viewer_header(
        &self,
    ) -> (
        libadwaita::HeaderBar,
        gtk4::Button,
        gtk4::DropDown,
        gtk4::Button,
        gtk4::Button,
        gtk4::Button,
    ) {
        let header = libadwaita::HeaderBar::new();

        let zoom_btn = gtk4::Button::from_icon_name("zoom-fit-best-symbolic");
        zoom_btn.set_tooltip_text(Some("Fit to Window (Ctrl+0)"));
        zoom_btn.add_css_class("flat");
        header.pack_end(&zoom_btn);

        let model_options = gtk4::StringList::new(&["Standard (Photo)", "Anime / Art"]);
        let upscale_model_dd = gtk4::DropDown::new(Some(model_options), None::<gtk4::Expression>);
        upscale_model_dd.set_selected(0);
        upscale_model_dd.set_tooltip_text(Some("Select the Real-ESRGAN model"));
        header.pack_end(&upscale_model_dd);

        let upscale_btn = gtk4::Button::with_label("AI Upscale");
        upscale_btn.set_tooltip_text(Some("Upscale selected image with AI"));
        upscale_btn.add_css_class("flat");
        header.pack_end(&upscale_btn);

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

        (
            header,
            zoom_btn,
            upscale_model_dd,
            upscale_btn,
            commit_btn,
            discard_btn,
        )
    }
}
