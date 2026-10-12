//! Focused crop-copy UI: preview coordinates map to oriented source pixels.
use crate::export::{CropConfig, CropRect};
use crate::ui::state::AppState;
use crate::ui::window::SharprWindow;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

pub fn present_crop_dialog(parent: &SharprWindow, source: PathBuf, state: Rc<RefCell<AppState>>) {
    let (tx, rx) = async_channel::bounded(1);
    let worker_source = source.clone();
    std::thread::spawn(move || {
        let _ = tx.send_blocking(crate::export::oriented_source_dimensions(&worker_source));
    });
    let weak = parent.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = rx.recv().await else { return };
        let Some(parent) = weak.upgrade() else { return };
        match result {
            Ok(dimensions) => build_dialog(&parent, source, dimensions, state),
            Err(error) => parent.add_toast(libadwaita::Toast::new(&format!(
                "Could not crop image: {error}"
            ))),
        }
    });
}

fn build_dialog(
    parent: &SharprWindow,
    source: PathBuf,
    dimensions: (u32, u32),
    state: Rc<RefCell<AppState>>,
) {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title("Crop Copy");
    dialog.set_content_width(760);
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let picture = gtk4::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk4::ContentFit::Contain);
    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&picture));
    let canvas = gtk4::DrawingArea::new();
    canvas.set_content_height(300);
    canvas.set_content_width(500);
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);
    overlay.add_overlay(&canvas);
    overlay.set_measure_overlay(&canvas, true);
    overlay.set_clip_overlay(&canvas, true);
    let selection = Rc::new(Cell::new(CropRect::centered_widescreen(dimensions)));
    let draw_selection = selection.clone();
    canvas.set_draw_func(move |_, cr, width, height| {
        let scale = (width as f64 / dimensions.0 as f64).min(height as f64 / dimensions.1 as f64);
        let ox = (width as f64 - dimensions.0 as f64 * scale) / 2.0;
        let oy = (height as f64 - dimensions.1 as f64 * scale) / 2.0;
        let r = draw_selection.get();
        let x = ox + r.x as f64 * scale;
        let y = oy + r.y as f64 * scale;
        let w = r.width as f64 * scale;
        let h = r.height as f64 * scale;
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
        for (x, y, w, h) in [
            (ox, oy, dimensions.0 as f64 * scale, y - oy),
            (
                ox,
                y + h,
                dimensions.0 as f64 * scale,
                dimensions.1 as f64 * scale - r.y as f64 * scale - h,
            ),
            (ox, y, x - ox, h),
            (
                x + w,
                y,
                dimensions.0 as f64 * scale - r.x as f64 * scale - w,
                h,
            ),
        ] {
            cr.rectangle(x, y, w.max(0.0), h.max(0.0));
            let _ = cr.fill();
        }
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
        cr.set_line_width(2.0);
        cr.rectangle(x, y, w, h);
        let _ = cr.stroke();
    });
    content.append(&overlay);
    let explanation = gtk4::Label::new(Some(
        "Drag to move the crop. Position and size are in displayed image pixels. Saves a new PNG.",
    ));
    explanation.set_wrap(true);
    explanation.add_css_class("dim-label");
    content.append(&explanation);
    let aspect = gtk4::DropDown::from_strings(&["16:9", "Free aspect"]);
    if dimensions.0 < 16 || dimensions.1 < 9 {
        aspect.set_selected(1);
    }
    let grid = gtk4::Grid::new();
    grid.set_column_spacing(8);
    grid.set_row_spacing(8);
    grid.attach(&gtk4::Label::new(Some("Aspect")), 0, 0, 1, 1);
    grid.attach(&aspect, 1, 0, 3, 1);
    let spins: Vec<_> = (0..4)
        .map(|i| {
            gtk4::SpinButton::with_range(
                if i < 2 { 0.0 } else { 1.0 },
                if i % 2 == 0 {
                    dimensions.0 as f64
                } else {
                    dimensions.1 as f64
                },
                1.0,
            )
        })
        .collect();
    let rect = selection.get();
    for (i, (label, value)) in [
        ("X", rect.x),
        ("Y", rect.y),
        ("Width", rect.width),
        ("Height", rect.height),
    ]
    .iter()
    .enumerate()
    {
        spins[i].set_value(*value as f64);
        spins[i].set_hexpand(true);
        let row = 1 + (i / 2) as i32;
        let col = ((i % 2) * 2) as i32;
        grid.attach(&gtk4::Label::new(Some(label)), col, row, 1, 1);
        grid.attach(&spins[i], col + 1, row, 1, 1);
    }
    let updating = Rc::new(Cell::new(false));
    let update: Rc<dyn Fn(usize)> = {
        let spins = spins.clone();
        let selection = selection.clone();
        let canvas = canvas.clone();
        let aspect = aspect.clone();
        let updating = updating.clone();
        Rc::new(move |changed| {
            if updating.replace(true) {
                return;
            }
            let mut w = spins[2].value_as_int() as u32;
            let mut h = spins[3].value_as_int() as u32;
            if aspect.selected() == 0 {
                let units = if changed == 3 { h / 9 } else { w / 16 };
                let maximum = (dimensions.0 / 16).min(dimensions.1 / 9);
                if maximum > 0 {
                    let units = units.max(1).min(maximum);
                    w = units * 16;
                    h = units * 9;
                }
            }
            w = w.clamp(1, dimensions.0);
            h = h.clamp(1, dimensions.1);
            let x = (spins[0].value_as_int() as u32).min(dimensions.0 - w);
            let y = (spins[1].value_as_int() as u32).min(dimensions.1 - h);
            selection.set(CropRect {
                x,
                y,
                width: w,
                height: h,
            });
            for (spin, value) in spins.iter().zip([x, y, w, h]) {
                spin.set_value(value as f64);
            }
            canvas.queue_draw();
            updating.set(false);
        })
    };
    for (i, spin) in spins.iter().enumerate() {
        let update = update.clone();
        spin.connect_value_changed(move |_| update(i));
    }
    {
        let update = update.clone();
        aspect.connect_selected_notify(move |_| update(2));
    }
    let drag = gtk4::GestureDrag::new();
    let origin = Rc::new(Cell::new(selection.get()));
    {
        let origin = origin.clone();
        let selection = selection.clone();
        drag.connect_drag_begin(move |_, _, _| origin.set(selection.get()));
    }
    {
        let canvas = canvas.clone();
        let origin = origin.clone();
        let spins = spins.clone();
        let updating = updating.clone();
        let update = update.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let scale = (canvas.width() as f64 / dimensions.0 as f64)
                .min(canvas.height() as f64 / dimensions.1 as f64);
            if scale <= 0.0 {
                return;
            }
            let r = origin.get();
            updating.set(true);
            spins[0]
                .set_value((r.x as f64 + dx / scale).clamp(0.0, (dimensions.0 - r.width) as f64));
            spins[1]
                .set_value((r.y as f64 + dy / scale).clamp(0.0, (dimensions.1 - r.height) as f64));
            updating.set(false);
            update(0);
        });
    }
    canvas.add_controller(drag);
    content.append(&grid);
    let destination = libadwaita::EntryRow::new();
    destination.set_title("Destination folder");
    destination.set_text(
        &state
            .borrow()
            .settings
            .resize_output_dir
            .clone()
            .unwrap_or_else(crate::export::default_resize_output_dir)
            .to_string_lossy(),
    );
    let choose = gtk4::Button::with_label("Choose…");
    destination.add_suffix(&choose);
    {
        let weak = parent.downgrade();
        let destination = destination.clone();
        choose.connect_clicked(move |_| {
            if let Some(parent) = weak.upgrade() {
                let chooser = gtk4::FileDialog::new();
                let destination = destination.clone();
                chooser.select_folder(Some(&parent), None::<&gio::Cancellable>, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            destination.set_text(&path.to_string_lossy());
                        }
                    }
                });
            }
        });
    }
    let group = libadwaita::PreferencesGroup::new();
    group.add(&destination);
    content.append(&group);
    let save = gtk4::Button::with_label("Create PNG Copy");
    save.add_css_class("suggested-action");
    save.set_halign(gtk4::Align::End);
    save.set_sensitive(false);

    let weak = parent.downgrade();
    let dialog_weak = dialog.downgrade();
    let save_source = source.clone();
    save.connect_clicked(move |_| {
        if destination.text().trim().is_empty() {
            return;
        }
        let folder = PathBuf::from(destination.text().as_str());
        let config = CropConfig {
            source: save_source.clone(),
            destination_dir: folder.clone(),
            rectangle: selection.get(),
        };
        state
            .borrow_mut()
            .settings
            .set_resize_output_dir(Some(folder));
        let operation = state.borrow().ops.add("Creating crop copy…");
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(crate::export::crop_copy(&config));
        });
        if let Some(dialog) = dialog_weak.upgrade() {
            dialog.close();
        }
        let weak = weak.clone();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else {
                operation.fail("Crop worker stopped");
                return;
            };
            match result {
                Ok(result) => {
                    operation.complete();
                    if let Some(parent) = weak.upgrade() {
                        parent.show_created_copy(result.output);
                    }
                }
                Err(error) => {
                    operation.fail(error.to_string());
                    if let Some(parent) = weak.upgrade() {
                        parent.add_toast(libadwaita::Toast::new(&format!("Crop failed: {error}")));
                    }
                }
            }
        });
    });
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(430);
    scroll.set_child(Some(&content));
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.append(&scroll);
    save.set_margin_bottom(12);
    save.set_margin_end(12);
    body.append(&save);
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent));
    let dialog_weak = dialog.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let result = async {
            let image = glycin::Loader::new(gio::File::for_path(source))
                .load()
                .await?;
            image.next_frame().await
        }
        .await;
        if dialog_weak.upgrade().is_none() {
            return;
        }
        match result {
            Ok(frame) => {
                picture.set_paintable(Some(&frame.texture()));
                save.set_sensitive(true);
            }
            Err(error) => explanation.set_text(&format!("Could not load crop preview: {error}")),
        }
    });
}
