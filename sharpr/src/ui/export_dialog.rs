//! "Export as WebP…" (`ROADMAP.md` Phase 6, decision D13): where the
//! selection goes and how. The choices are remembered; the export itself
//! runs in the background (`SharprWindow::start_webp_export`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::config::settings::{wallpapers_dir, WebpExportSettings};
use crate::export::batch::WebpOptions;
use crate::ui::window::SharprWindow;

fn folder_label(dir: &std::path::Path) -> String {
    let home = dirs::home_dir();
    match home.as_deref().and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

pub fn present(window: &SharprWindow, sources: Vec<PathBuf>) {
    if sources.is_empty() {
        return;
    }
    let saved = window.app_state().borrow().settings.webp_export.clone();
    let folder = Rc::new(RefCell::new(
        saved.dir.clone().unwrap_or_else(wallpapers_dir),
    ));

    let count = sources.len();
    let dialog = libadwaita::AlertDialog::new(
        Some(&if count == 1 {
            "Export Image as WebP".to_string()
        } else {
            format!("Export {count} Images as WebP")
        }),
        Some("Copies are new files with unique names; the originals are never changed."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("export", "Export");
    dialog.set_response_appearance("export", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("export"));
    dialog.set_close_response("cancel");
    dialog.set_content_width(560);

    let group = libadwaita::PreferencesGroup::new();
    let folder_row = libadwaita::ActionRow::new();
    folder_row.set_title("Folder");
    folder_row.set_subtitle(&folder_label(&folder.borrow()));
    let wallpapers = gtk4::Button::with_label("Wallpapers");
    wallpapers.set_valign(gtk4::Align::Center);
    wallpapers.add_css_class("flat");
    wallpapers.set_tooltip_text(Some("Export to ~/Pictures/Wallpapers"));
    {
        let folder = folder.clone();
        let row = folder_row.clone();
        wallpapers.connect_clicked(move |_| {
            *folder.borrow_mut() = wallpapers_dir();
            row.set_subtitle(&folder_label(&folder.borrow()));
        });
    }
    let choose = gtk4::Button::with_label("Choose…");
    choose.set_valign(gtk4::Align::Center);
    {
        let folder = folder.clone();
        let row = folder_row.clone();
        let window = window.downgrade();
        choose.connect_clicked(move |_| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let chooser = gtk4::FileDialog::new();
            chooser.set_initial_folder(Some(&gio::File::for_path(&*folder.borrow())));
            let folder = folder.clone();
            let row = row.clone();
            chooser.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
                if let Some(path) = result.ok().and_then(|file| file.path()) {
                    row.set_subtitle(&folder_label(&path));
                    *folder.borrow_mut() = path;
                }
            });
        });
    }
    folder_row.add_suffix(&wallpapers);
    folder_row.add_suffix(&choose);
    group.add(&folder_row);

    let lossless = libadwaita::SwitchRow::new();
    lossless.set_title("Lossless");
    lossless.set_subtitle("Keeps every pixel; larger files");
    lossless.set_active(saved.lossless);
    group.add(&lossless);
    let quality = libadwaita::SpinRow::with_range(1.0, 100.0, 1.0);
    quality.set_title("Quality");
    quality.set_value(f64::from(saved.quality));
    quality.set_sensitive(!saved.lossless);
    lossless
        .bind_property("active", &quality, "sensitive")
        .invert_boolean()
        .build();
    group.add(&quality);

    let fit = libadwaita::SwitchRow::new();
    fit.set_title("Fit Within");
    fit.set_subtitle("Shrinks larger images to fit; never enlarges");
    fit.set_active(saved.fit);
    group.add(&fit);
    let width = libadwaita::SpinRow::with_range(16.0, 16_383.0, 1.0);
    width.set_title("Width");
    width.set_value(f64::from(saved.fit_width));
    let height = libadwaita::SpinRow::with_range(16.0, 16_383.0, 1.0);
    height.set_title("Height");
    height.set_value(f64::from(saved.fit_height));
    for row in [&width, &height] {
        row.set_sensitive(saved.fit);
        fit.bind_property("active", row, "sensitive").build();
        group.add(row);
    }
    dialog.set_extra_child(Some(&group));

    let window_weak = window.downgrade();
    dialog.connect_response(None, move |_, response| {
        if response != "export" {
            return;
        }
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        let destination = folder.borrow().clone();
        let settings = WebpExportSettings {
            dir: (destination != wallpapers_dir()).then(|| destination.clone()),
            lossless: lossless.is_active(),
            quality: quality.value() as u8,
            fit: fit.is_active(),
            fit_width: width.value() as u32,
            fit_height: height.value() as u32,
        };
        window
            .app_state()
            .borrow_mut()
            .settings
            .set_webp_export(settings.clone());
        window.start_webp_export(
            sources.clone(),
            WebpOptions {
                destination,
                lossless: settings.lossless,
                quality: settings.quality,
                fit: settings
                    .fit
                    .then_some((settings.fit_width, settings.fit_height)),
            },
        );
    });
    dialog.present(Some(window));
}
