use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::ui::window::SharprWindow;

// ---------------------------------------------------------------------------
// GObject subclass boilerplate
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SharprApplication {}

    #[glib::object_subclass]
    impl ObjectSubclass for SharprApplication {
        const NAME: &'static str = "SharprApplication";
        type Type = super::SharprApplication;
        type ParentType = libadwaita::Application;
    }

    impl ObjectImpl for SharprApplication {}

    impl ApplicationImpl for SharprApplication {
        fn activate(&self) {
            self.parent_activate();
            let app = self.obj();

            // Reuse the existing window if one is already open.
            if let Some(window) = app.active_window() {
                window.present();
                return;
            }

            let window = SharprWindow::new(app.upcast_ref::<libadwaita::Application>());
            window.present();
        }

        fn startup(&self) {
            self.parent_startup();
            // Set up application-level actions here in later phases (e.g. quit, about).
        }
    }

    impl GtkApplicationImpl for SharprApplication {}
    impl AdwApplicationImpl for SharprApplication {}
}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

glib::wrapper! {
    pub struct SharprApplication(ObjectSubclass<imp::SharprApplication>)
        @extends libadwaita::Application, gtk4::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl SharprApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", "com.example.Sharpr")
            .property("flags", gio::ApplicationFlags::FLAGS_NONE)
            .build()
    }
}
