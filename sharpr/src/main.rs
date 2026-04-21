mod app;
mod bench;
mod config;
mod duplicates;
mod metadata;
mod model;
mod ops;
mod quality;
mod tags;
mod thumbnails;
mod ui;
mod upscale;

use app::SharprApplication;
use gtk4::{gio, prelude::*};

// Embed the compiled GResource bundle into the binary at build time.
fn register_resources() {
    gio::resources_register_include!("sharpr.gresource")
        .expect("Failed to register GResource bundle");
}

fn main() -> glib::ExitCode {
    bench::init();

    // Initialise rexiv2 (gexiv2 C library) once at startup.
    rexiv2::initialize().expect("Failed to initialise gexiv2");
    register_resources();

    let app = SharprApplication::new();
    app.run()
}
