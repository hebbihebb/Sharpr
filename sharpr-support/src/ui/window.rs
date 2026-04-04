use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::gio;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::config::AppSettings;
use crate::model::{ImageEntry, LibraryManager};
use crate::thumbnails::ThumbnailWorker;
use crate::ui::filmstrip::FilmstripPane;
use crate::ui::sidebar::SidebarPane;
use crate::ui::viewer::ViewerPane;

// ---------------------------------------------------------------------------
// Shared application state (main thread only, Rc<RefCell<>>)
// ---------------------------------------------------------------------------

pub struct AppState {
    pub library: LibraryManager,
    pub settings: AppSettings,
}

impl AppState {
    fn new() -> Self {
        Self {
            library: LibraryManager::new(),
            settings: AppSettings::load(),
        }
    }
}

// ---------------------------------------------------------------------------
// GObject subclass
// ---------------------------------------------------------------------------

mod imp {
    use super::*;
    use async_channel::Receiver;
    use crate::thumbnails::worker::ThumbnailResult;

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

        // Sidebar folder selection → scan library → refresh filmstrip.
        {
            let filmstrip_c = filmstrip.clone();
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            sidebar.connect_folder_selected(move |path| {
                state_c.borrow_mut().library.scan_folder(&path);
                // Persist last folder.
                state_c.borrow_mut().settings.last_folder = Some(path.clone());
                state_c.borrow().settings.save();

                filmstrip_c.refresh();
                viewer_c.clear();
            });
        }

        // Filmstrip selection → viewer load.
        {
            let viewer_c = viewer.clone();
            let state_c = state.clone();
            filmstrip.connect_image_selected(move |index| {
                state_c.borrow_mut().library.selected_index = Some(index);
                let entry: Option<ImageEntry> = state_c.borrow().library.entry_at(index);
                if let Some(entry) = entry {
                    viewer_c.load_image(entry.path());
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

        let viewer_toolbar = libadwaita::ToolbarView::new();
        viewer_toolbar.add_top_bar(&self.build_viewer_header());
        viewer_toolbar.set_content(Some(&viewer));
        viewer_toolbar.set_top_bar_style(libadwaita::ToolbarStyle::Raised);

        let viewer_page = libadwaita::NavigationPage::builder()
            .title("Image Library")
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
        // Restore last folder
        // -----------------------------------------------------------------------
        let last_folder = state.borrow().settings.last_folder.clone();
        if let Some(folder) = last_folder {
            if folder.is_dir() {
                state.borrow_mut().library.scan_folder(&folder);
                filmstrip.refresh();
            }
        }
    }

    /// Drain thumbnail results from the worker pool on the GLib main context.
    /// For each result: construct a `MemoryTexture` on the main thread,
    /// find the matching `ImageEntry` in the store, and update it.
    fn start_thumbnail_poll(
        &self,
        state: Rc<RefCell<AppState>>,
        filmstrip: FilmstripPane,
    ) {
        let result_rx = self.imp().result_rx.borrow().clone();
        let Some(rx) = result_rx else { return };

        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                use gdk4::{MemoryFormat, MemoryTexture};
                use glib::Bytes;

                let bytes = Bytes::from_owned(result.rgba_bytes);
                let texture = MemoryTexture::new(
                    result.width as i32,
                    result.height as i32,
                    MemoryFormat::R8g8b8a8,
                    &bytes,
                    (result.width * 4) as usize,
                );

                // Find the entry by path and update it.
                // Hold the borrow only for the search, then release before signalling.
                let found = {
                    let st = state.borrow();
                    let store = &st.library.store;
                    let n = store.n_items();
                    let mut found: Option<(u32, ImageEntry)> = None;
                    for i in 0..n {
                        if let Some(entry) = store.item(i).and_downcast::<ImageEntry>() {
                            if entry.path() == result.path {
                                entry.set_thumbnail(texture.clone().upcast());
                                found = Some((i, entry));
                                break;
                            }
                        }
                    }
                    found.map(|(i, _)| i)
                };

                // Cache the texture in LibraryManager (needs mut borrow).
                {
                    state
                        .borrow_mut()
                        .library
                        .insert_thumbnail(result.path.clone(), texture.upcast());
                }

                // Notify GtkListView that item `i` changed (triggers re-bind).
                if let Some(idx) = found {
                    state.borrow().library.store.items_changed(idx, 1, 1);
                }
            }
        });
    }

    fn build_viewer_header(&self) -> libadwaita::HeaderBar {
        let header = libadwaita::HeaderBar::new();

        let zoom_btn = gtk4::Button::from_icon_name("zoom-fit-best-symbolic");
        zoom_btn.set_tooltip_text(Some("Fit to Window (Ctrl+0)"));
        zoom_btn.add_css_class("flat");
        header.pack_end(&zoom_btn);

        header
    }
}
