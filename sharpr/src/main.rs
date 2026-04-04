mod app;
mod config;
mod metadata;
mod model;
mod thumbnails;
mod ui;
mod upscale;

use app::SharprApplication;
use gtk4::prelude::*;

fn main() -> glib::ExitCode {
    // Initialise rexiv2 (gexiv2 C library) once at startup.
    rexiv2::initialize().expect("Failed to initialise gexiv2");

    let app = SharprApplication::new();
    app.run()
}
