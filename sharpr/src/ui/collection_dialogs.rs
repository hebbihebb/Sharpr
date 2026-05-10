use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::config::FolderMode;
use crate::library_index::normalize_collection_tag;
use crate::ui::sidebar::SidebarPane;
use crate::ui::window::{AppState, ViewScope};

pub(super) fn parse_collection_tags_input(input: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for tag in input.split(',') {
        let tag = normalize_collection_tag(tag);
        if !tag.is_empty() && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

const SWATCH_PALETTE: &[&str] = &[
    "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
];

fn parse_hex_color(color: &str) -> Option<(f64, f64, f64)> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 {
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
                .unwrap_or_else(|| parse_hex_color("#57e389").unwrap());
            cr.set_source_rgb(r, g, b);
            append_rounded_rect(cr, 1.0, 1.0, 18.0, 18.0, 4.0);
            let _ = cr.fill();

            if selected_for_draw.borrow().as_deref() == Some(color_string.as_str()) {
                cr.set_source_rgb(1.0, 1.0, 1.0);
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
    initial_extra_tags: String,
    state: Rc<RefCell<AppState>>,
    toast_overlay: libadwaita::ToastOverlay,
    refresh: F,
) where
    F: Fn() + Clone + 'static,
{
    let dialog = libadwaita::AlertDialog::new(Some("New Collection"), None);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some("Collection name"));
    name_entry.set_text(&initial_name);
    let (color_swatch_row, selected_color) = build_color_swatch_row(None);
    let tags_entry = gtk4::Entry::new();
    tags_entry.set_placeholder_text(Some("Extra tags, comma separated"));
    tags_entry.set_text(&initial_extra_tags);
    let info = gtk4::Label::new(Some("The collection name is also used as a tag."));
    info.add_css_class("dim-label");
    info.set_wrap(true);
    info.set_halign(gtk4::Align::Start);
    let entry_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    entry_box.set_margin_top(6);
    entry_box.append(&name_entry);
    entry_box.append(&color_swatch_row);
    entry_box.append(&info);
    entry_box.append(&tags_entry);
    dialog.set_extra_child(Some(&entry_box));
    let state_d = state.clone();
    let toast_d = toast_overlay.clone();
    let refresh_d = refresh.clone();
    let name_clone = name_entry.clone();
    let tags_clone = tags_entry.clone();
    let selected_color_clone = selected_color.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "create" {
            return;
        }
        let name = name_clone.text().to_string();
        let extra_tags = parse_collection_tags_input(tags_clone.text().as_str());
        let selected_color = selected_color_clone.borrow().clone();
        let idx = state_d.borrow().library_index.clone();
        if let Some(idx) = idx {
            let started = std::time::Instant::now();
            let library_id = state_d
                .borrow()
                .settings
                .active_library()
                .map(|l| l.id.clone())
                .unwrap_or_default();
            match idx.create_collection(
                &library_id,
                None,
                &name,
                &extra_tags,
                selected_color.as_deref(),
                None,
            ) {
                Ok(coll) => {
                    crate::bench_event!(
                        "collection.create",
                        serde_json::json!({
                            "collection_id": coll.id,
                            "name": coll.name,
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    refresh_d();
                    toast_d.add_toast(libadwaita::Toast::new(&format!(
                        "Collection \u{201c}{}\u{201d} created",
                        coll.name
                    )));
                }
                Err(e) => {
                    toast_d.add_toast(libadwaita::Toast::new(&format!(
                        "Could not create collection: {e}"
                    )));
                }
            }
        }
    });
    dialog.present(Some(&window));
}

pub(super) fn show_new_library_dialog(
    window: gtk4::Window,
    state: Rc<RefCell<AppState>>,
    sidebar: SidebarPane,
    toast_overlay: libadwaita::ToastOverlay,
) {
    let dialog = libadwaita::AlertDialog::new(Some("Create Library"), None);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);

    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some("Library name"));
    let root_entry = gtk4::Entry::new();
    root_entry.set_editable(false);
    root_entry.set_hexpand(true);
    let choose_button = gtk4::Button::with_label("Choose…");
    let top_level = gtk4::CheckButton::with_label("Top level only");
    let drill_down = gtk4::CheckButton::with_label("Drill into subfolders");
    drill_down.set_group(Some(&top_level));
    top_level.set_active(true);
    let root_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    root_row.append(&root_entry);
    root_row.append(&choose_button);

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    box_.set_margin_top(6);
    box_.append(&name_entry);
    box_.append(&root_row);
    box_.append(&top_level);
    box_.append(&drill_down);
    dialog.set_extra_child(Some(&box_));

    {
        let root_entry_c = root_entry.clone();
        let window_c = window.clone();
        choose_button.connect_clicked(move |_| {
            let chooser = gtk4::FileDialog::new();
            chooser.set_title("Choose Library Root");
            let root_entry_inner = root_entry_c.clone();
            chooser.select_folder(Some(&window_c), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        root_entry_inner.set_text(&path.to_string_lossy());
                    }
                }
            });
        });
    }

    dialog.connect_response(None, move |_, response| {
        if response != "create" {
            return;
        }
        let root = PathBuf::from(root_entry.text().as_str());
        let folder_mode = if drill_down.is_active() {
            FolderMode::DrillDown
        } else {
            FolderMode::TopLevel
        };
        let created = {
            let mut st = state.borrow_mut();
            let result =
                st.settings
                    .add_library(name_entry.text().as_str(), root.clone(), folder_mode);
            match result {
                Ok(id) => {
                    st.settings.set_active_library(&id);
                    st.disabled_folders = st
                        .settings
                        .active_library()
                        .map(|library| library.ignored_folders.clone())
                        .unwrap_or_default();
                    Ok(())
                }
                Err(err) => Err(err),
            }
        };
        match created {
            Ok(()) => {
                sidebar.refresh_active_library(state.clone());
                toast_overlay.add_toast(libadwaita::Toast::new("Library created"));
            }
            Err(err) => toast_overlay.add_toast(libadwaita::Toast::new(&err)),
        }
    });
    dialog.present(Some(&window));
}

pub(super) fn switch_active_library(
    library_id: &str,
    state: &Rc<RefCell<AppState>>,
    sidebar: &SidebarPane,
) {
    {
        let mut st = state.borrow_mut();
        st.settings.set_active_library(library_id);
        st.disabled_folders = st
            .settings
            .active_library()
            .map(|library| library.ignored_folders.clone())
            .unwrap_or_default();
        st.selected_paths.clear();
        st.scope = ViewScope::Search;
        let _ = st.library.load_virtual(&[]);
    }
    sidebar.imp().collapsed_folder_paths.borrow_mut().clear();
    sidebar.refresh_active_library(state.clone());
}
