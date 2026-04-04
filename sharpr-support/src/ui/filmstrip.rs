use std::cell::RefCell;
use std::rc::Rc;

use gdk4::Paintable;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::gio;
use libadwaita::prelude::*;

use crate::model::ImageEntry;
use crate::ui::window::AppState;

type ImageSelectedCallback = Box<dyn Fn(u32) + 'static>;

// ---------------------------------------------------------------------------
// FilmstripPane
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct FilmstripPane {
        pub toolbar_view: libadwaita::ToolbarView,
        /// The list view displays images in a vertical strip.
        pub list_view: gtk4::ListView,
        /// Single-selection model — we update its underlying GListModel on refresh.
        pub selection_model: gtk4::SingleSelection,
        pub image_selected_cb: RefCell<Option<ImageSelectedCallback>>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
    }

    impl Default for FilmstripPane {
        fn default() -> Self {
            let factory = gtk4::SignalListItemFactory::new();

            // Setup: create widget hierarchy for one recycled slot.
            // NOTE: In GTK 4.12+, factory signals pass &glib::Object; downcast to ListItem.
            factory.connect_setup(|_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>()
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

            // Bind: populate the recycled widget with data from the ImageEntry.
            factory.connect_bind(|_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>()
                    .expect("factory object must be ListItem");

                let entry: ImageEntry = match list_item.item().and_downcast::<ImageEntry>() {
                    Some(e) => e,
                    None => return,
                };
                let item_box: gtk4::Box = match list_item.child().and_downcast::<gtk4::Box>() {
                    Some(b) => b,
                    None => return,
                };

                // First child = GtkPicture, last child = GtkLabel.
                let picture = item_box.first_child().and_downcast::<gtk4::Picture>();
                let label = item_box.last_child().and_downcast::<gtk4::Label>();

                if let Some(lbl) = label {
                    lbl.set_text(&entry.filename());
                }
                if let Some(pic) = picture {
                    match entry.thumbnail() {
                        Some(ref texture) => {
                            pic.set_paintable(Some(texture.upcast_ref::<Paintable>()))
                        }
                        None => pic.set_paintable(None::<&Paintable>),
                    }
                }
            });

            // Unbind: release paintable references so recycled slots are clean.
            factory.connect_unbind(|_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>()
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

            let selection_model = gtk4::SingleSelection::new(None::<gio::ListStore>);
            selection_model.set_can_unselect(false);

            let list_view = gtk4::ListView::new(Some(selection_model.clone()), Some(factory));
            list_view.set_orientation(gtk4::Orientation::Vertical);
            list_view.add_css_class("navigation-sidebar");

            Self {
                toolbar_view: libadwaita::ToolbarView::new(),
                list_view,
                selection_model,
                image_selected_cb: RefCell::new(None),
                state: RefCell::new(None),
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

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        scroll.set_child(Some(&imp.list_view));

        imp.toolbar_view.set_content(Some(&scroll));
        imp.toolbar_view.set_parent(self);

        // Connect selection change: notify the window when user picks an image.
        let widget_weak = self.downgrade();
        imp.selection_model
            .connect_selected_notify(move |selection| {
                let Some(widget) = widget_weak.upgrade() else {
                    return;
                };
                let index = selection.selected();
                widget.emit_image_selected(index);
            });
    }

    /// Swap the list view's underlying model to the current LibraryManager store.
    /// Call this after `library.scan_folder()`.
    pub fn refresh(&self) {
        let imp = self.imp();
        let state_opt = imp.state.borrow();
        let Some(ref state_rc) = *state_opt else { return };

        let store = state_rc.borrow().library.store.clone();
        imp.selection_model.set_model(Some(&store));
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
