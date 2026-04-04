use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gdk4::Paintable;
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
        imp.selection_model.set_model(Some(&store));
        self.schedule_visible_thumbnails();
    }

    /// Give the filmstrip a sender to the thumbnail worker so the factory bind
    /// callback can enqueue requests for unthumb'd entries.
    pub fn set_thumbnail_sender(&self, tx: async_channel::Sender<WorkerRequest>) {
        *self.imp().thumbnail_tx.borrow_mut() = Some(tx);
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
        let Some(state_rc) = imp.state.borrow().as_ref().cloned() else {
            return;
        };

        let adjustment = imp.scroll.vadjustment();
        let visible_start = (adjustment.value() / ESTIMATED_ROW_HEIGHT).floor().max(0.0) as u32;
        let visible_rows = (adjustment.page_size() / ESTIMATED_ROW_HEIGHT)
            .ceil()
            .max(1.0) as u32;
        let range_start = visible_start.saturating_sub(BUFFER_ROWS);
        let range_end = visible_start
            .saturating_add(visible_rows)
            .saturating_add(BUFFER_ROWS);

        let mut pending = imp.pending_thumbnails.borrow_mut();
        let state = state_rc.borrow();
        let image_count = state.library.image_count();
        let capped_end = range_end.min(image_count);

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

            // Non-blocking scheduling keeps scrolling responsive.
            if tx.try_send(WorkerRequest::Thumbnail(path.clone())).is_err() {
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
