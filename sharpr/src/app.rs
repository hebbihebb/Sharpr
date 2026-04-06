use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
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

            // Show splash, then present the main window after 1.8 s.
            // The delay gives warm-cache thumbnail workers time to populate
            // visible rows before the UI appears.
            let splash = gtk4::Window::builder()
                .decorated(false)
                .resizable(false)
                .default_width(600)
                .default_height(400)
                .build();
            let splash_pic = gtk4::Picture::for_resource("/io/github/hebbihebb/Sharpr/splash.png");
            splash_pic.set_content_fit(gtk4::ContentFit::Fill);
            splash.set_child(Some(&splash_pic));
            splash.present();

            let window = SharprWindow::new(app.upcast_ref::<libadwaita::Application>());
            glib::timeout_add_local_once(std::time::Duration::from_millis(1800), move || {
                splash.close();
                window.present();
            });
        }

        fn startup(&self) {
            self.parent_startup();
            let app = self.obj();
            let about = gio::SimpleAction::new("about", None);
            {
                let app_weak = app.downgrade();
                about.connect_activate(move |_, _| {
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    let dialog = adw::AboutDialog::new();
                    dialog.set_application_name("Sharpr");
                    dialog.set_application_icon("io.github.hebbihebb.Sharpr");
                    dialog.set_developer_name("Sharpr Contributors");
                    dialog.set_version("0.1.0");
                    dialog.set_license_type(gtk4::License::Gpl30Only);
                    dialog.set_copyright("© 2026 Sharpr Contributors");
                    if let Some(win) = app.active_window() {
                        dialog.present(Some(&win));
                    }
                });
            }
            app.add_action(&about);
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
            .property("application-id", "io.github.hebbihebb.Sharpr")
            .property("flags", gio::ApplicationFlags::FLAGS_NONE)
            .build()
    }
}
