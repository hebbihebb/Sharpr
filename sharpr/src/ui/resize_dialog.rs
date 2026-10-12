use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::export::{
    default_resize_output_dir, height_for_locked_width, oriented_source_dimensions, resize_copy,
    width_for_locked_height, ResizeConfig, ResizeFormat,
};
use crate::ui::state::AppState;
use crate::ui::window::SharprWindow;

pub fn present_resize_dialog(
    parent: &SharprWindow,
    source: PathBuf,
    state: Rc<RefCell<AppState>>,
    refresh_saved_folder: Rc<dyn Fn(PathBuf)>,
) {
    present_copy_dialog(parent, source, state, refresh_saved_folder, false);
}

pub fn present_webp_dialog(
    parent: &SharprWindow,
    source: PathBuf,
    state: Rc<RefCell<AppState>>,
    refresh_saved_folder: Rc<dyn Fn(PathBuf)>,
) {
    present_copy_dialog(parent, source, state, refresh_saved_folder, true);
}

fn present_copy_dialog(
    parent: &SharprWindow,
    source: PathBuf,
    state: Rc<RefCell<AppState>>,
    refresh_saved_folder: Rc<dyn Fn(PathBuf)>,
    webp: bool,
) {
    let operation = state.borrow().ops.add("Reading image size…");
    let (tx, rx) = async_channel::bounded(1);
    let source_for_worker = source.clone();
    std::thread::spawn(move || {
        let result = oriented_source_dimensions(&source_for_worker);
        let _ = tx.send_blocking(result);
    });

    let parent_weak = parent.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = rx.recv().await else {
            operation.fail("Could not read image size");
            return;
        };
        let Some(parent) = parent_weak.upgrade() else {
            return;
        };
        match result {
            Ok(dimensions) => {
                operation.complete();
                build_dialog(
                    &parent,
                    source,
                    dimensions,
                    state,
                    refresh_saved_folder,
                    webp,
                );
            }
            Err(error) => {
                operation.fail(error.to_string());
                parent.add_toast(libadwaita::Toast::new(&format!(
                    "Could not resize image: {error}"
                )));
            }
        }
    });
}

fn build_dialog(
    parent: &SharprWindow,
    source: PathBuf,
    source_dimensions: (u32, u32),
    state: Rc<RefCell<AppState>>,
    refresh_saved_folder: Rc<dyn Fn(PathBuf)>,
    webp: bool,
) {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title(if webp { "WebP Copy" } else { "Resize Copy" });
    dialog.set_content_width(480);

    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let dimensions = gtk4::Grid::new();
    dimensions.set_column_spacing(12);
    dimensions.set_row_spacing(8);
    let width = gtk4::SpinButton::with_range(1.0, 65_535.0, 1.0);
    let height = gtk4::SpinButton::with_range(1.0, 65_535.0, 1.0);
    width.set_value(source_dimensions.0 as f64);
    height.set_value(source_dimensions.1 as f64);
    width.set_hexpand(true);
    height.set_hexpand(true);
    dimensions.attach(&gtk4::Label::new(Some("Width")), 0, 0, 1, 1);
    dimensions.attach(&width, 1, 0, 1, 1);
    dimensions.attach(&gtk4::Label::new(Some("Height")), 0, 1, 1, 1);
    dimensions.attach(&height, 1, 1, 1, 1);

    let lock = gtk4::CheckButton::with_label("Lock aspect ratio");
    lock.set_active(true);
    let updating = Rc::new(Cell::new(false));
    {
        let height = height.clone();
        let lock = lock.clone();
        let updating = updating.clone();
        width.connect_value_changed(move |width| {
            if !lock.is_active() || updating.replace(true) {
                return;
            }
            height.set_value(
                height_for_locked_width(source_dimensions, width.value_as_int() as u32) as f64,
            );
            updating.set(false);
        });
    }
    {
        let width = width.clone();
        let lock = lock.clone();
        let updating = updating.clone();
        height.connect_value_changed(move |height| {
            if !lock.is_active() || updating.replace(true) {
                return;
            }
            width.set_value(
                width_for_locked_height(source_dimensions, height.value_as_int() as u32) as f64,
            );
            updating.set(false);
        });
    }

    let format_row = libadwaita::ComboRow::new();
    format_row.set_title("Format");
    format_row.set_model(Some(&gtk4::StringList::new(&[
        "JPEG", "PNG", "WebP", "JPEG XL",
    ])));

    if webp {
        format_row.set_selected(2);
        format_row.set_sensitive(false);
    }

    let quality = libadwaita::SpinRow::with_range(1.0, 100.0, 1.0);
    quality.set_title("Quality");
    quality.set_subtitle("Used for lossy formats");
    quality.set_value(90.0);
    {
        let quality = quality.clone();
        format_row.connect_selected_notify(move |row| {
            quality.set_sensitive(row.selected() != 1);
        });
    }

    let destination = libadwaita::EntryRow::new();
    destination.set_title("Destination folder");
    let initial_destination = state
        .borrow()
        .settings
        .resize_output_dir
        .clone()
        .unwrap_or_else(default_resize_output_dir);
    destination.set_text(&initial_destination.to_string_lossy());
    let choose = gtk4::Button::with_label("Choose…");
    destination.add_suffix(&choose);
    {
        let destination = destination.clone();
        let parent_weak = parent.downgrade();
        choose.connect_clicked(move |_| {
            let Some(parent) = parent_weak.upgrade() else {
                return;
            };
            let chooser = gtk4::FileDialog::new();
            chooser.set_title("Choose Resize Destination");
            let destination = destination.clone();
            chooser.select_folder(
                Some(parent.upcast_ref::<gtk4::Window>()),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            destination.set_text(&path.to_string_lossy());
                        }
                    }
                },
            );
        });
    }

    let resize = gtk4::Button::with_label(if webp {
        "Create WebP Copy"
    } else {
        "Create Resized Copy"
    });
    resize.add_css_class("suggested-action");
    resize.set_halign(gtk4::Align::End);
    let webp_limit = gtk4::Label::new(Some(
        "WebP supports up to 16,383 pixels per side. Reduce larger SeedVR outputs here before converting.",
    ));
    webp_limit.set_wrap(true);
    webp_limit.set_xalign(0.0);
    webp_limit.add_css_class("dim-label");
    let validate = Rc::new({
        let width = width.clone();
        let height = height.clone();
        let format = format_row.clone();
        let resize = resize.clone();
        let note = webp_limit.clone();
        move || {
            let is_webp = format.selected() == 2;
            note.set_visible(is_webp);
            resize.set_sensitive(
                !is_webp
                    || (width.value_as_int() as u32 <= crate::export::WEBP_MAX_DIMENSION
                        && height.value_as_int() as u32 <= crate::export::WEBP_MAX_DIMENSION),
            );
        }
    });
    validate();
    for dimension in [&width, &height] {
        let validate = validate.clone();
        dimension.connect_value_changed(move |_| validate());
    }
    format_row.connect_selected_notify(move |_| validate());
    {
        let dialog_weak = dialog.downgrade();
        let parent_weak = parent.downgrade();
        let source = source.clone();
        let state = state.clone();
        let destination_c = destination.clone();
        let format_row_c = format_row.clone();
        let quality_c = quality.clone();
        resize.connect_clicked(move |_| {
            let destination_dir = PathBuf::from(destination_c.text().as_str());
            let format = match format_row_c.selected() {
                1 => ResizeFormat::Png,
                2 => ResizeFormat::Webp,
                3 => ResizeFormat::Jxl,
                _ => ResizeFormat::Jpeg,
            };
            let config = ResizeConfig {
                source: source.clone(),
                destination_dir: destination_dir.clone(),
                target_width: width.value_as_int() as u32,
                target_height: height.value_as_int() as u32,
                format,
                quality: quality_c.value() as u8,
            };
            state
                .borrow_mut()
                .settings
                .set_resize_output_dir(Some(destination_dir));
            let operation = state.borrow().ops.add(if webp {
                "Creating WebP copy…"
            } else {
                "Creating resized copy…"
            });
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.close();
            }
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(resize_copy(&config));
            });
            let parent_weak = parent_weak.clone();
            let refresh_saved_folder = refresh_saved_folder.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(result) = rx.recv().await else {
                    operation.fail("Resize worker stopped");
                    return;
                };
                match result {
                    Ok(result) => {
                        operation.complete();
                        if let Some(parent) = parent_weak.upgrade() {
                            let name = result
                                .output
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "resized copy".to_string());
                            parent.add_toast(libadwaita::Toast::new(&format!("Created {name}")));
                        }
                        refresh_saved_folder(result.output);
                    }
                    Err(error) => {
                        operation.fail(error.to_string());
                        if let Some(parent) = parent_weak.upgrade() {
                            parent.add_toast(libadwaita::Toast::new(&format!(
                                "Resize failed: {error}"
                            )));
                        }
                    }
                }
            });
        });
    }

    content.append(&dimensions);
    content.append(&lock);
    let preferences = libadwaita::PreferencesGroup::new();
    preferences.add(&format_row);
    preferences.add(&quality);
    preferences.add(&destination);
    content.append(&preferences);
    content.append(&webp_limit);
    content.append(&resize);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent));
}
