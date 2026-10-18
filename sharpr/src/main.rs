mod app;
mod bench;
mod buckets;
mod config;
mod copies;
mod db_version;
mod display_scale;
mod export;
mod file_jobs;
mod file_ops;
mod fingerprint;
mod image_pipeline;
mod jxl;
mod library_index;
mod library_overview;
mod metadata;
mod model;
mod ops;
mod palette;
mod quality;
mod tags;
mod thumbnails;
mod ui;
mod visual_search;

use app::SharprApplication;
use gtk4::{gio, prelude::*};

// Embed the compiled GResource bundle into the binary at build time.
fn register_resources() {
    gio::resources_register_include!("sharpr.gresource")
        .expect("Failed to register GResource bundle");
}

fn main() -> glib::ExitCode {
    bench::init();
    register_resources();

    let app = SharprApplication::new();
    app.run()
}
