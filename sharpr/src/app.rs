use gdk4::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::ui::window::SharprWindow;

const APP_DISPLAY_VERSION: &str = env!("SHARPR_DISPLAY_VERSION");

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
            // Register the GResource prefix so GTK's icon theme finds the
            // bundled app icon (icons/hicolor/512x512/apps/…) at runtime.
            if let Some(display) = gdk4::Display::default() {
                gtk4::IconTheme::for_display(&display)
                    .add_resource_path("/io/github/hebbihebb/Sharpr");
            }
            gtk4::Window::set_default_icon_name("io.github.hebbihebb.Sharpr");
            let app = self.obj();
            let about = gio::SimpleAction::new("about", None);
            {
                let app_weak = app.downgrade();
                about.connect_activate(move |_, _| {
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    // Re-ensure icon theme has the resource path registered on
                    // the active display before the dialog resolves the icon.
                    if let Some(display) = gdk4::Display::default() {
                        gtk4::IconTheme::for_display(&display)
                            .add_resource_path("/io/github/hebbihebb/Sharpr");
                    }
                    let dialog = adw::AboutDialog::new();
                    dialog.set_application_name("Sharpr");
                    dialog.set_application_icon("io.github.hebbihebb.Sharpr");
                    dialog.set_developer_name("Sharpr Contributors");
                    dialog.set_version(APP_DISPLAY_VERSION);
                    dialog.set_license_type(gtk4::License::Gpl30Only);
                    dialog.set_copyright("© 2026 Sharpr Contributors");
                    dialog.set_website("https://github.com/hebbihebb/Sharpr");
                    dialog.set_issue_url("https://github.com/hebbihebb/Sharpr/issues");
                    dialog.add_credit_section(
                        Some("Built with"),
                        &[
                            "GTK4 — https://gtk.org",
                            "Libadwaita — https://gnome.pages.gitlab.gnome.org/libadwaita",
                            "Rust — https://rust-lang.org",
                        ],
                    );
                    dialog.add_credit_section(
                        Some("Libraries"),
                        &[
                            "image-rs — Image decoding",
                            "SQLite + rusqlite — Tag storage",
                            "rexiv2 / GExiv2 — EXIF metadata",
                        ],
                    );
                    dialog.add_credit_section(
                        Some("AI Tag Suggestions"),
                        &[
                            "ResNet-18 — ONNX Model Zoo",
                            "tract-onnx — Pure-Rust ONNX inference",
                        ],
                    );
                    dialog.add_credit_section(Some("AI Upscaling"), &["RealESRGAN-NCNN-Vulkan"]);
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
