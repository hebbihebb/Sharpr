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

// Retain D25's click-away cancellation for this quick export form. This is
// the earlier dialog handler, scoped to export now that other pickers use
// popovers. The window capture sees backdrop clicks without a native grab.
fn close_on_click_outside(window: &SharprWindow, dialog: &libadwaita::Dialog) {
    let click = gtk4::GestureClick::new();
    click.set_name(Some("export-sheet-outside"));
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    let weak_window = window.downgrade();
    let weak_dialog = dialog.downgrade();
    click.connect_pressed(move |gesture, _, x, y| {
        let (Some(window), Some(dialog)) = (weak_window.upgrade(), weak_dialog.upgrade()) else {
            return;
        };
        if window.visible_dialog().as_ref() != Some(&dialog) {
            return;
        }
        let point = gtk4::graphene::Point::new(x as f32, y as f32);
        let inside = dialog
            .child()
            .and_then(|card| card.compute_bounds(&window))
            .is_some_and(|bounds| bounds.contains_point(&point));
        if !inside {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            dialog.close();
        }
    });
    window.add_controller(click.clone());
    let window = window.downgrade();
    dialog.connect_closed(move |_| {
        if let Some(window) = window.upgrade() {
            window.remove_controller(&click);
        }
    });
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
    // Keep the form on the main window's surface while the native chooser
    // opens and closes. Recreating a grabbing Wayland popup here can fail.
    let dialog = libadwaita::Dialog::new();
    dialog.add_css_class("sharpr-sheet");
    dialog.set_title(&heading.label());
    dialog.set_content_width(460);
    dialog.set_presentation_mode(libadwaita::DialogPresentationMode::Floating);

    let group = libadwaita::PreferencesGroup::new();
    let folder_row = libadwaita::ActionRow::new();
    folder_row.set_title("Folder");
    folder_row.set_subtitle(&folder_label(&folder.borrow()));
    folder_row.set_subtitle_lines(1);
    folder_row.set_tooltip_text(Some(&folder.borrow().display().to_string()));
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
            row.set_tooltip_text(Some(&folder.borrow().display().to_string()));
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
                    row.set_tooltip_text(Some(&path.display().to_string()));
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
    let cancel = gtk4::Button::with_label("Cancel");
    let export = gtk4::Button::with_label("Export");
    export.add_css_class("suggested-action");
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    buttons.set_homogeneous(true);
    buttons.append(&cancel);
    buttons.append(&export);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.set_width_request(380);
    content.append(&note);
    content.append(&group);
    content.append(&buttons);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    note.set_wrap(true);
    let header = libadwaita::HeaderBar::new();
    header.set_title_widget(Some(&heading));
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();
    let view = libadwaita::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&export));
    {
        let dialog = dialog.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        });
    }

    let window_weak = window.downgrade();
    let dialog_weak = dialog.downgrade();
    export.connect_clicked(move |_| {
        if let Some(dialog) = dialog_weak.upgrade() {
            dialog.close();
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
    close_on_click_outside(window, &dialog);
    dialog.present(Some(window));
}
