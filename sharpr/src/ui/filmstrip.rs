use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use gdk4::Paintable;
use glib::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::model::ImageEntry;
use crate::thumbnails::worker::WorkerRequest;
use crate::ui::window::AppState;

type ImageSelectedCallback = Box<dyn Fn(u32) + 'static>;

// ---------------------------------------------------------------------------
// FilmstripPane
// ---------------------------------------------------------------------------

mod imp {
    use super::*;
    use async_channel::Sender;

    pub struct FilmstripPane {
        pub toolbar_view: libadwaita::ToolbarView,
        pub scroll: gtk4::ScrolledWindow,
        pub list_view: gtk4::ListView,
        pub selection_model: gtk4::SingleSelection,
        pub image_selected_cb: RefCell<Option<ImageSelectedCallback>>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        /// Sender to the background thumbnail worker. Set by the window after spawn.
        pub thumbnail_tx: RefCell<Option<Sender<WorkerRequest>>>,
        /// Shared generation counter — bumped on every folder switch so workers
        /// skip stale requests from the previous folder immediately.
        pub thumbnail_gen: RefCell<Option<Arc<std::sync::atomic::AtomicU64>>>,
        /// Paths already queued or being generated for the current folder.
        pub pending_thumbnails: RefCell<HashSet<PathBuf>>,
    }

    impl Default for FilmstripPane {
        fn default() -> Self {
            // Factory is set up in build_ui so the bind closure can capture a
            // weak reference to the widget (needed to reach thumbnail_tx).
            let selection_model = gtk4::SingleSelection::new(None::<gio::ListStore>);
            selection_model.set_can_unselect(false);

            // No factory yet; set_factory is called from build_ui.
            let list_view =
                gtk4::ListView::new(Some(selection_model.clone()), None::<gtk4::ListItemFactory>);
            list_view.set_orientation(gtk4::Orientation::Vertical);
            list_view.add_css_class("navigation-sidebar");

            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                scroll: gtk4::ScrolledWindow::new(),
                list_view,
                selection_model,
                image_selected_cb: RefCell::new(None),
                state: RefCell::new(None),
                thumbnail_tx: RefCell::new(None),
                thumbnail_gen: RefCell::new(None),
                pending_thumbnails: RefCell::new(HashSet::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FilmstripPane {
        const NAME: &'static str = "SharprFilmstripPane";
        type Type = super::FilmstripPane;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for FilmstripPane {
        fn dispose(&self) {
            self.toolbar_view.unparent();
        }
    }

    impl WidgetImpl for FilmstripPane {}
}

glib::wrapper! {
    pub struct FilmstripPane(ObjectSubclass<imp::FilmstripPane>)
        @extends gtk4::Widget;
}

impl FilmstripPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        let widget: Self = glib::Object::new();
        *widget.imp().state.borrow_mut() = Some(state);
        widget.build_ui();
        widget
    }

    fn build_ui(&self) {
        let imp = self.imp();

        let header = libadwaita::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        imp.toolbar_view.add_top_bar(&header);

        // -----------------------------------------------------------------------
        // GtkSignalListItemFactory — set up here so closures can capture self.
        // NOTE: In GTK 4.12+, factory signals pass &glib::Object; downcast to ListItem.
        // -----------------------------------------------------------------------
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");

            let item_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            item_box.set_margin_top(4);
            item_box.set_margin_bottom(4);
            item_box.set_margin_start(4);
            item_box.set_margin_end(4);

            let picture = gtk4::Picture::new();
            picture.set_content_fit(gtk4::ContentFit::Cover);
            picture.set_size_request(148, 100);
            picture.add_css_class("thumbnail-frame");

            let label = gtk4::Label::new(None);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            label.set_max_width_chars(18);
            label.add_css_class("caption");

            item_box.append(&picture);
            item_box.append(&label);
            list_item.set_child(Some(&item_box));
        });

        // Bind: populate widgets for recycled rows. Scheduling happens from the
        // viewport rather than per-bind so we don't eagerly decode the whole folder.
        factory.connect_bind(move |_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");

            let entry: ImageEntry = match list_item.item().and_downcast::<ImageEntry>() {
                Some(e) => e,
                None => return,
            };
            let item_box: gtk4::Box = match list_item.child().and_downcast::<gtk4::Box>() {
                Some(b) => b,
                None => return,
            };

            let picture = item_box.first_child().and_downcast::<gtk4::Picture>();
            let label = item_box.last_child().and_downcast::<gtk4::Label>();

            if let Some(lbl) = label {
                lbl.set_text(&entry.filename());
            }
            if let Some(pic) = picture {
                match entry.thumbnail() {
                    Some(ref texture) => pic.set_paintable(Some(texture.upcast_ref::<Paintable>())),
                    None => pic.set_paintable(None::<&Paintable>),
                }
            }
        });

        // Unbind: clear references so recycled widgets don't hold stale paintables.
        factory.connect_unbind(|_, obj| {
            let list_item = obj
                .downcast_ref::<gtk4::ListItem>()
                .expect("factory object must be ListItem");
            if let Some(item_box) = list_item.child().and_downcast::<gtk4::Box>() {
                if let Some(pic) = item_box.first_child().and_downcast::<gtk4::Picture>() {
                    pic.set_paintable(None::<&Paintable>);
                }
                if let Some(lbl) = item_box.last_child().and_downcast::<gtk4::Label>() {
                    lbl.set_text("");
                }
            }
        });

        imp.list_view.set_factory(Some(&factory));

        imp.scroll
            .set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        imp.scroll.set_vexpand(true);
        imp.scroll.set_hexpand(true);
        imp.scroll.set_child(Some(&imp.list_view));

        imp.toolbar_view.set_content(Some(&imp.scroll));
        imp.toolbar_view.set_parent(self);

        let widget_weak = self.downgrade();
        imp.scroll.vadjustment().connect_value_changed(move |_| {
            if let Some(widget) = widget_weak.upgrade() {
                widget.schedule_visible_thumbnails();
            }
        });

        // Connect selection change. Guard against INVALID_LIST_POSITION (u32::MAX),
        // which GTK emits when the model is empty or no item is selected.
        let widget_weak = self.downgrade();
        imp.selection_model
            .connect_selected_notify(move |selection| {
                let index = selection.selected();
                if index == gtk4::INVALID_LIST_POSITION {
                    return;
                }
                if let Some(widget) = widget_weak.upgrade() {
                    widget.emit_image_selected(index);
                }
            });
    }

    /// Swap the list view's underlying model to the current LibraryManager store.
    /// Call after `library.scan_folder()`.
    pub fn refresh(&self) {
        let imp = self.imp();
        let state_opt = imp.state.borrow();
        let Some(ref state_rc) = *state_opt else {
            return;
        };
        imp.pending_thumbnails.borrow_mut().clear();
        let store = state_rc.borrow().library.store.clone();
        // Clear selection first so selected_notify fires even if new folder
        // lands on the same position (e.g. both at index 0).
        imp.selection_model.set_model(None::<&gio::ListStore>);
        imp.selection_model.set_model(Some(&store));
        // Schedule immediately (uses 20-row fallback if layout not done yet),
        // then re-schedule once after GTK finishes layout to pick up the real
        // page_size and catch any rows the fallback missed.
        self.schedule_visible_thumbnails();
        let w = self.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(filmstrip) = w.upgrade() {
                filmstrip.schedule_visible_thumbnails();
            }
        });
    }

    /// Give the filmstrip a sender to the thumbnail worker and the shared
    /// generation counter. Call once after the worker is spawned.
    pub fn set_thumbnail_sender(
        &self,
        tx: async_channel::Sender<WorkerRequest>,
        gen: Arc<std::sync::atomic::AtomicU64>,
    ) {
        *self.imp().thumbnail_tx.borrow_mut() = Some(tx);
        *self.imp().thumbnail_gen.borrow_mut() = Some(gen);
        self.schedule_visible_thumbnails();
    }

    pub fn mark_thumbnail_ready(&self, path: &Path) {
        self.imp().pending_thumbnails.borrow_mut().remove(path);
    }

    pub fn schedule_visible_thumbnails(&self) {
        const ESTIMATED_ROW_HEIGHT: f64 = 120.0;
        const BUFFER_ROWS: u32 = 6;

        let imp = self.imp();
        let Some(tx) = imp.thumbnail_tx.borrow().as_ref().cloned() else {
            return;
        };
        let gen = imp
            .thumbnail_gen
            .borrow()
            .as_ref()
            .map_or(0, |a| a.load(Ordering::Relaxed));
        let Some(state_rc) = imp.state.borrow().as_ref().cloned() else {
            return;
        };

        let adjustment = imp.scroll.vadjustment();
        let visible_start = (adjustment.value() / ESTIMATED_ROW_HEIGHT).floor().max(0.0) as u32;
        // If page_size is 0 the widget hasn't been laid out yet; assume a
        // generous default (20 rows) so thumbnails are queued on first load.
        let page_size = adjustment.page_size();
        let visible_rows = if page_size > 0.0 {
            (page_size / ESTIMATED_ROW_HEIGHT).ceil() as u32
        } else {
            20
        };
        let range_start = visible_start.saturating_sub(BUFFER_ROWS);
        let range_end = visible_start
            .saturating_add(visible_rows)
            .saturating_add(BUFFER_ROWS);

        let mut pending = imp.pending_thumbnails.borrow_mut();
        let image_count = state_rc.borrow().library.image_count();
        let capped_end = range_end.min(image_count);

        // Collect disk-cache hits and worker-needed paths separately so we
        // can drop the state borrow before calling items_changed.
        let mut disk_hits: Vec<(u32, std::path::PathBuf, std::path::PathBuf)> = Vec::new();
        let mut worker_paths: Vec<std::path::PathBuf> = Vec::new();

        {
            let state = state_rc.borrow();
            for index in range_start..capped_end {
                let Some(entry) = state.library.entry_at(index) else {
                    continue;
                };
                if entry.thumbnail().is_some() {
                    continue;
                }
                let path = entry.path();
                if !pending.insert(path.clone()) {
                    continue;
                }
                // Check disk cache — avoids a worker round-trip for previously-seen images.
                if let Some(cache_path) = crate::thumbnails::cache::thumbnail_cache_path(&path) {
                    if cache_path.exists() {
                        disk_hits.push((index, path, cache_path));
                        continue;
                    }
                }
                worker_paths.push(path);
            }
        }

        // Load disk-cached thumbnails inline — a 160 px PNG decodes in ~1-2 ms.
        for (index, path, cache_path) in disk_hits {
            if let Ok(img) = image::open(&cache_path) {
                let rgba = img.into_rgba8();
                let (w, h) = (rgba.width(), rgba.height());
                let gbytes = glib::Bytes::from_owned(rgba.into_raw());
                let texture = gdk4::MemoryTexture::new(
                    w as i32,
                    h as i32,
                    gdk4::MemoryFormat::R8g8b8a8,
                    &gbytes,
                    (w * 4) as usize,
                );
                let texture: gdk4::Texture = texture.upcast();
                // Set on entry and warm the LRU; drop state borrow before items_changed.
                {
                    let state = state_rc.borrow();
                    if let Some(entry) = state.library.entry_at(index) {
                        entry.set_thumbnail(texture.clone());
                    }
                }
                state_rc.borrow_mut().library.insert_thumbnail(path.clone(), texture);
                state_rc.borrow().library.store.items_changed(index, 1, 1);
                pending.remove(&path);
            }
        }

        // Send remaining cache-miss paths to the worker pool.
        for path in worker_paths {
            if tx.try_send(WorkerRequest::Thumbnail { path: path.clone(), gen }).is_err() {
                pending.remove(&path);
                break;
            }
        }
    }

    /// Programmatically select `index` in the list view and scroll it into view.
    /// Used by keyboard navigation (Alt+Left/Right) in the window.
    pub fn select_index(&self, index: u32) {
        let imp = self.imp();
        imp.selection_model.set_selected(index);
        // Scroll the filmstrip so the selected row is visible.
        imp.list_view.scroll_to(
            index,
            gtk4::ListScrollFlags::SELECT | gtk4::ListScrollFlags::FOCUS,
            None,
        );
    }

    pub fn connect_image_selected<F: Fn(u32) + 'static>(&self, f: F) {
        *self.imp().image_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    fn emit_image_selected(&self, index: u32) {
        if let Some(cb) = self.imp().image_selected_cb.borrow().as_ref() {
            cb(index);
        }
    }
}
