use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::ui::state::{AppState, ViewScope};

pub(super) fn run_collection_write<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    complete: impl FnOnce(Result<T, String>) + 'static,
) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::MainContext::default().spawn_local(async move {
        if let Ok(result) = rx.recv().await {
            complete(result);
        }
    });
}

use crate::palette::BUCKETS as SWATCH_PALETTE;

fn parse_hex_color(color: &str) -> Option<(f64, f64, f64)> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    ))
}

fn append_rounded_rect(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let right = x + width;
    let bottom = y + height;
    let degrees = std::f64::consts::PI / 180.0;

    cr.new_sub_path();
    cr.arc(
        right - radius,
        y + radius,
        radius,
        -90.0 * degrees,
        0.0 * degrees,
    );
    cr.arc(
        right - radius,
        bottom - radius,
        radius,
        0.0 * degrees,
        90.0 * degrees,
    );
    cr.arc(
        x + radius,
        bottom - radius,
        radius,
        90.0 * degrees,
        180.0 * degrees,
    );
    cr.arc(
        x + radius,
        y + radius,
        radius,
        180.0 * degrees,
        270.0 * degrees,
    );
    cr.close_path();
}

pub(super) fn build_color_swatch_row(
    selected: Option<&str>,
) -> (gtk4::Widget, Rc<RefCell<Option<String>>>) {
    let selected_color = Rc::new(RefCell::new(selected.map(str::to_string)));
    let flowbox = gtk4::FlowBox::new();
    flowbox.set_max_children_per_line(8);
    flowbox.set_row_spacing(4);
    flowbox.set_column_spacing(4);
    flowbox.set_selection_mode(gtk4::SelectionMode::None);
    flowbox.set_halign(gtk4::Align::Start);
    let swatches: Rc<RefCell<Vec<gtk4::DrawingArea>>> = Rc::new(RefCell::new(Vec::new()));

    for color in SWATCH_PALETTE {
        let button = gtk4::Button::new();
        button.add_css_class("flat");
        let swatch = gtk4::DrawingArea::new();
        swatch.set_content_width(20);
        swatch.set_content_height(20);

        let color_string = (*color).to_string();
        let selected_for_draw = selected_color.clone();
        swatch.set_draw_func(move |_, cr, _, _| {
            let (r, g, b) = parse_hex_color(&color_string)
                .unwrap_or_else(|| parse_hex_color(SWATCH_PALETTE[0]).unwrap());
            cr.set_source_rgb(r, g, b);
            append_rounded_rect(cr, 1.0, 1.0, 18.0, 18.0, 4.0);
            let _ = cr.fill();

            if selected_for_draw.borrow().as_deref() == Some(color_string.as_str()) {
                super::look::cairo_color(cr, crate::palette::WHITE);
                cr.set_line_width(2.0);
                append_rounded_rect(cr, 3.0, 3.0, 14.0, 14.0, 3.0);
                let _ = cr.stroke();
            }
        });

        button.set_child(Some(&swatch));
        let selected_for_click = selected_color.clone();
        let swatches_for_click = swatches.clone();
        let color_for_click = (*color).to_string();
        button.connect_clicked(move |_| {
            *selected_for_click.borrow_mut() = Some(color_for_click.clone());
            for swatch in swatches_for_click.borrow().iter() {
                swatch.queue_draw();
            }
        });
        swatches.borrow_mut().push(swatch);
        flowbox.insert(&button, -1);
    }

    (flowbox.upcast(), selected_color)
}

pub(super) fn show_new_collection_dialog<F>(
    window: gtk4::Window,
    initial_name: String,
    state: Rc<RefCell<AppState>>,
    toast_overlay: libadwaita::ToastOverlay,
    refresh: F,
) where
    F: Fn() + Clone + 'static,
{
    let dialog = libadwaita::AlertDialog::new(Some("New Bucket"), None);
    dialog.add_css_class("sharpr-sheet");
    dialog.add_response("create", "Create");
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some("Bucket name"));
    name_entry.set_text(&initial_name);
    let (color_swatch_row, selected_color) = build_color_swatch_row(None);
    let entry_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    entry_box.set_margin_top(6);
    entry_box.append(&name_entry);
    entry_box.append(&color_swatch_row);
    dialog.set_extra_child(Some(&entry_box));
    let state_d = state.clone();
    let toast_d = toast_overlay.clone();
    let refresh_d = refresh.clone();
    let name_clone = name_entry.clone();
    let selected_color_clone = selected_color.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "create" {
            return;
        }
        let name = name_clone.text().to_string();
        let selected_color = selected_color_clone.borrow().clone();
        let idx = state_d.borrow().library_index.clone();
        if let Some(idx) = idx {
            let library_id = state_d
                .borrow()
                .settings
                .active_library_id
                .clone()
                .unwrap_or_default();
            let refresh = refresh_d.clone();
            let toast = toast_d.clone();
            run_collection_write(
                move || {
                    idx.create_collection(
                        &library_id,
                        None,
                        &name,
                        &[],
                        selected_color.as_deref(),
                        None,
                    )
                    .map_err(|e| e.to_string())
                },
                move |result| match result {
                    Ok(collection) => {
                        crate::bench_event!(
                            "collection.create",
                            serde_json::json!({"id":collection.id})
                        );
                        refresh();
                        toast.add_toast(libadwaita::Toast::new(&format!(
                            "Created bucket {}",
                            collection.name
                        )));
                    }
                    Err(error) => toast.add_toast(libadwaita::Toast::new(&format!(
                        "Could not create bucket: {error}"
                    ))),
                },
            );
        }
    });
    dialog.present(Some(&window));
}

pub(super) fn show_rename_collection_dialog<F>(
    window: gtk4::Window,
    id: i64,
    state: Rc<RefCell<AppState>>,
    toast_overlay: libadwaita::ToastOverlay,
    refresh: F,
) where
    F: Fn() + Clone + 'static,
{
    let Some(idx) = state.borrow().library_index.clone() else {
        return;
    };
    let Some(collection) = idx.collection(id).ok().flatten() else {
        return;
    };

    let dialog = libadwaita::AlertDialog::new(Some("Rename Bucket"), None);
    dialog.add_css_class("sharpr-sheet");
    dialog.add_response("save", "Save");
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("save", libadwaita::ResponseAppearance::Suggested);

    let name_entry = gtk4::Entry::new();
    name_entry.set_text(&collection.name);
    name_entry.select_region(0, -1);
    name_entry.set_activates_default(true);
    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    box_.set_margin_top(6);
    box_.append(&name_entry);
    dialog.set_extra_child(Some(&box_));

    let state_d = state.clone();
    let toast_d = toast_overlay.clone();
    let refresh_d = refresh.clone();
    let name_clone = name_entry.clone();
    let window_weak = window.downgrade();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let (index, library_id) = {
            let st = state_d.borrow();
            (
                st.library_index.clone(),
                st.settings.active_library_id.clone(),
            )
        };
        let Some(index) = index else {
            return;
        };
        let new_name = name_clone.text().to_string();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                crate::buckets::rename_bucket(&index, id, &new_name)?;
                crate::bench_event!("collection.rename.finish", serde_json::json!({"id":id}));
                Ok(())
            })();
            let _ = tx.send_blocking(result);
        });
        let state = state_d.clone();
        let toast = toast_d.clone();
        let refresh = refresh_d.clone();
        let weak = window_weak.clone();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            match result {
                Ok(()) => {
                    refresh();
                    let active = {
                        let st = state.borrow();
                        st.scope == ViewScope::Collection(id)
                            && st.settings.active_library_id == library_id
                    };
                    if active {
                        if let Some(window) = weak
                            .upgrade()
                            .and_then(|w| w.downcast::<super::window::SharprWindow>().ok())
                        {
                            window.refresh_current_bucket(id);
                        }
                    }
                }
                Err(error) => toast.add_toast(libadwaita::Toast::new(&format!(
                    "Could not rename bucket: {error}"
                ))),
            }
        });
    });
    dialog.present(Some(&window));
    name_entry.grab_focus();
}

pub(super) fn show_change_color_dialog<F>(
    window: gtk4::Window,
    id: i64,
    state: Rc<RefCell<AppState>>,
    toast_overlay: libadwaita::ToastOverlay,
    refresh: F,
) where
    F: Fn() + Clone + 'static,
{
    let Some(idx) = state.borrow().library_index.clone() else {
        return;
    };
    let Some(collection) = idx.collection(id).ok().flatten() else {
        return;
    };

    let dialog = libadwaita::AlertDialog::new(Some("Change Bucket Color"), None);
    dialog.add_css_class("sharpr-sheet");
    dialog.add_response("save", "Save");
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("save", libadwaita::ResponseAppearance::Suggested);

    let (color_swatch_row, selected_color) = build_color_swatch_row(collection.color.as_deref());
    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    box_.set_margin_top(6);
    box_.append(&color_swatch_row);
    dialog.set_extra_child(Some(&box_));

    let state_d = state.clone();
    let toast_d = toast_overlay.clone();
    let refresh_d = refresh.clone();
    let selected_color_clone = selected_color.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let Some(idx) = state_d.borrow().library_index.clone() else {
            return;
        };
        let color = selected_color_clone.borrow().clone();
        let refresh = refresh_d.clone();
        let toast = toast_d.clone();
        run_collection_write(
            move || {
                let before = idx
                    .collection(id)
                    .map_err(|e| e.to_string())?
                    .ok_or("Bucket no longer exists")?;
                idx.update_collection(
                    id,
                    &before.name,
                    &before.extra_tags,
                    color.as_deref(),
                    before.icon_name.as_deref(),
                )
                .map_err(|e| e.to_string())
            },
            move |result| match result {
                Ok(()) => {
                    crate::bench_event!("collection.color", serde_json::json!({"id":id}));
                    refresh();
                }
                Err(error) => toast.add_toast(libadwaita::Toast::new(&format!(
                    "Could not change bucket color: {error}"
                ))),
            },
        );
    });
    dialog.present(Some(&window));
}
