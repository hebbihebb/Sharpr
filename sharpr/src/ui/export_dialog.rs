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
    // Look prototype: a compact popup at the pointer instead of a centred
    // card (I7).
    let heading = gtk4::Label::new(Some(&if count == 1 {
        "Export Image as WebP".to_string()
    } else {
        format!("Export {count} Images as WebP")
    }));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    let note = gtk4::Label::new(Some("New files with unique names; originals never change."));
    note.add_css_class("dim-label");
    note.add_css_class("caption");
    note.set_xalign(0.0);
    let popover_cell: Rc<glib::WeakRef<gtk4::Popover>> = Rc::new(glib::WeakRef::new());

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
        let popover_cell = popover_cell.clone();
        choose.connect_clicked(move |_| {
            let Some(window) = window.upgrade() else {
                return;
            };
            // The folder chooser is another window; the popup stays open.
            let popover = popover_cell.upgrade();
            if let Some(popover) = popover.as_ref() {
                popover.set_autohide(false);
            }
            let chooser = gtk4::FileDialog::new();
            chooser.set_initial_folder(Some(&gio::File::for_path(&*folder.borrow())));
            let folder = folder.clone();
            let row = row.clone();
            chooser.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
                if let Some(path) = result.ok().and_then(|file| file.path()) {
                    row.set_subtitle(&folder_label(&path));
                    *folder.borrow_mut() = path;
                }
                if let Some(popover) = popover.as_ref() {
                    popover.set_autohide(true);
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
    let cancel = gtk4::Button::with_label("Cancel");
    let export = gtk4::Button::with_label("Export");
    export.add_css_class("suggested-action");
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    buttons.set_homogeneous(true);
    buttons.append(&cancel);
    buttons.append(&export);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.set_width_request(380);
    content.append(&heading);
    content.append(&note);
    content.append(&group);
    content.append(&buttons);
    let popover = window.popover_at_pointer(&content);
    popover.set_default_widget(Some(&export));
    popover_cell.set(Some(&popover));
    {
        let popover = popover.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
        });
    }

    let window_weak = window.downgrade();
    let popover_weak = popover.downgrade();
    export.connect_clicked(move |_| {
        if let Some(popover) = popover_weak.upgrade() {
            popover.popdown();
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
}
