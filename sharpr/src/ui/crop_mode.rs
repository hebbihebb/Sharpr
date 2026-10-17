//! Crop Copy in the viewer at full size (`ROADMAP.md` Phase 6): the image
//! fills the viewer with a selection to move and resize, 16:9 or free, and
//! saves a new copy in the source format. The side panels make way while
//! cropping; F11 makes it fullscreen. Originals are only read.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::export::{Corner, CropConfig, CropRect};
use crate::ui::window::SharprWindow;

/// The longest edge of a preview decoded on a worker.
const PREVIEW_EDGE: u32 = 2560;
/// How close to a corner, in screen pixels, a drag resizes.
const HANDLE: f64 = 18.0;

#[derive(Clone, Copy)]
enum Drag {
    Move(CropRect),
    Resize(Corner, CropRect),
}

struct Crop {
    source: PathBuf,
    /// The source's size and modification time when cropping started.
    stamp: (u64, i64),
    dims: (u32, u32),
    rect: Cell<CropRect>,
    aspect: Cell<Option<(u32, u32)>>,
    drag: Cell<Option<Drag>>,
}

/// Where the image sits on a `width`×`height` canvas: offset and scale.
fn placement(dims: (u32, u32), width: f64, height: f64) -> (f64, f64, f64) {
    let scale = (width / f64::from(dims.0)).min(height / f64::from(dims.1));
    (
        (width - f64::from(dims.0) * scale) / 2.0,
        (height - f64::from(dims.1) * scale) / 2.0,
        scale,
    )
}

/// Starts cropping `source`: reads its displayed size and a preview on a
/// worker (reusing the viewer's picture when it shows this image), then
/// covers the viewer. The selection starts on `frame` (fractions of the
/// image: x, y, width, height) when given, such as the viewer's 16:9 guide
/// (D38), or centred.
pub fn start(window: &SharprWindow, source: PathBuf, frame: Option<(f64, f64, f64, f64)>) {
    let Some(viewer) = window.imp_viewer() else {
        return;
    };
    let shown = (viewer.current_path().as_ref() == Some(&source))
        .then(|| viewer.imp().picture.paintable())
        .flatten();
    let need_preview = shown.is_none();
    // A later request, or moving to another image, makes this one stale.
    let request = window.imp().crop_request.get().wrapping_add(1);
    window.imp().crop_request.set(request);
    window.imp().crop_preparing.set(true);
    let viewing = viewer.current_path();
    let (tx, rx) = async_channel::bounded(1);
    let worker_source = source.clone();
    std::thread::spawn(move || {
        // Displayed size, file stamp, and a preview when one is needed.
        type Prepared = ((u32, u32), (u64, i64), Option<(u32, u32, Vec<u8>)>);
        let result = (|| -> Result<Prepared, String> {
            let stamp = std::fs::metadata(&worker_source)
                .map(|m| crate::fingerprint::file_stamp(&m))
                .map_err(|e| e.to_string())?;
            let dims = crate::export::oriented_source_dimensions(&worker_source)
                .map_err(|e| e.to_string())?;
            let preview = if need_preview {
                // The viewer can't show it, or shows another image: decode
                // an oriented preview here instead.
                let image =
                    crate::export::decode_oriented(&worker_source).map_err(|e| e.to_string())?;
                let (w, h) = crate::visual_search::prepare::fit_within(
                    image.width(),
                    image.height(),
                    PREVIEW_EDGE,
                );
                let rgba = image
                    .resize(w, h, image::imageops::FilterType::Triangle)
                    .to_rgba8();
                Some((rgba.width(), rgba.height(), rgba.into_raw()))
            } else {
                None
            };
            Ok((dims, stamp, preview))
        })();
        let _ = tx.send_blocking(result);
    });
    let weak = window.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = rx.recv().await else { return };
        let Some(window) = weak.upgrade() else { return };
        if window.imp().crop_request.get() != request {
            return;
        }
        window.imp().crop_preparing.set(false);
        let moved_on = window
            .imp_viewer()
            .is_none_or(|viewer| viewer.current_path() != viewing);
        if moved_on || window.crop_session().is_some() {
            return;
        }
        match result {
            Ok((dims, stamp, preview)) => {
                let paintable = shown.or_else(|| {
                    preview.map(|(w, h, rgba)| {
                        gdk4::MemoryTexture::new(
                            w as i32,
                            h as i32,
                            gdk4::MemoryFormat::R8g8b8a8,
                            &glib::Bytes::from_owned(rgba),
                            w as usize * 4,
                        )
                        .upcast()
                    })
                });
                if let Some(paintable) = paintable {
                    open(&window, source, dims, stamp, frame, &paintable);
                }
            }
            Err(error) => {
                window.add_toast(libadwaita::Toast::new(&format!("Could not crop: {error}")))
            }
        }
    });
}

fn open(
    window: &SharprWindow,
    source: PathBuf,
    dims: (u32, u32),
    stamp: (u64, i64),
    frame: Option<(f64, f64, f64, f64)>,
    paintable: &gdk4::Paintable,
) {
    let Some(viewer) = window.imp_viewer() else {
        return;
    };
    let aspect = (dims.0 >= 16 && dims.1 >= 9).then_some((16, 9));
    let crop = Rc::new(Crop {
        source,
        stamp,
        dims,
        rect: Cell::new(match frame {
            Some(frame) => CropRect::framed(dims, frame, aspect),
            None => CropRect::centered(dims, aspect),
        }),
        aspect: Cell::new(aspect),
        drag: Cell::new(None),
    });

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("background");
    root.set_focusable(true);
    let stage = gtk4::Overlay::new();
    stage.set_vexpand(true);
    // Look prototype: a neutral canvas in both themes (I14).
    stage.add_css_class("sharpr-canvas");
    let picture = gtk4::Picture::for_paintable(paintable);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_margin_top(12);
    picture.set_margin_bottom(12);
    picture.set_margin_start(12);
    picture.set_margin_end(12);
    stage.set_child(Some(&picture));
    let canvas = gtk4::DrawingArea::new();
    canvas.set_margin_top(12);
    canvas.set_margin_bottom(12);
    canvas.set_margin_start(12);
    canvas.set_margin_end(12);
    stage.add_overlay(&canvas);
    root.append(&stage);

    let controls = crate::ui::window::mode_header::ModeControls::new();
    let bar = &controls.start;
    let aspects = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    aspects.add_css_class("linked");
    let wide = gtk4::ToggleButton::with_label("16:9");
    let free = gtk4::ToggleButton::with_label("Free");
    free.set_group(Some(&wide));
    wide.set_active(aspect.is_some());
    free.set_active(aspect.is_none());
    wide.set_sensitive(aspect.is_some());
    aspects.append(&wide);
    aspects.append(&free);
    bar.append(&aspects);
    let size = gtk4::Label::new(None);
    size.add_css_class("dim-label");
    size.add_css_class("numeric");
    controls.center.append(&size);
    // Look prototype: the keys that work here, in one quiet line (D26).
    let spacer = gtk4::Label::new(None);
    spacer.set_markup(
        "drag to move · drag a corner to resize   <b>Enter</b> save   <b>Esc</b> cancel",
    );
    spacer.add_css_class("sharpr-keys");
    spacer.set_wrap(true);
    spacer.set_margin_bottom(6);
    root.append(&spacer);
    let bar = &controls.end;
    let destination = Rc::new(RefCell::new(
        window
            .app_state()
            .borrow()
            .settings
            .webp_export
            .dir
            .clone()
            .unwrap_or_else(crate::config::settings::wallpapers_dir),
    ));
    let folder = gtk4::Button::new();
    folder.add_css_class("flat");
    folder.set_tooltip_text(Some("Folder for the cropped copy"));
    let folder_label = |path: &std::path::Path| {
        format!(
            "To {}…",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        )
    };
    let folder_text = gtk4::Label::new(Some(&folder_label(&destination.borrow())));
    folder_text.set_ellipsize(pango::EllipsizeMode::Middle);
    folder_text.set_max_width_chars(18);
    folder.set_child(Some(&folder_text));
    folder.set_tooltip_text(Some(&format!(
        "Destination: {}",
        destination.borrow().display()
    )));
    bar.append(&folder);
    let cancel = gtk4::Button::with_label("Cancel");
    let save = gtk4::Button::with_label("Save Copy");
    save.add_css_class("suggested-action");
    save.set_tooltip_text(Some(
        "Save the selection as a new copy in the source format (Enter)",
    ));
    bar.append(&cancel);
    bar.append(&save);

    let update_size: Rc<dyn Fn()> = {
        let crop = crop.clone();
        let size = size.clone();
        Rc::new(move || {
            let r = crop.rect.get();
            size.set_text(&format!("{} × {}", r.width, r.height));
        })
    };
    update_size();

    {
        let crop = crop.clone();
        canvas.set_draw_func(move |_, cr, width, height| {
            let (ox, oy, scale) = placement(crop.dims, f64::from(width), f64::from(height));
            let (iw, ih) = (
                f64::from(crop.dims.0) * scale,
                f64::from(crop.dims.1) * scale,
            );
            let r = crop.rect.get();
            let (x, y) = (ox + f64::from(r.x) * scale, oy + f64::from(r.y) * scale);
            let (w, h) = (f64::from(r.width) * scale, f64::from(r.height) * scale);
            // Shade everything outside the selection.
            super::look::cairo_color(cr, crate::palette::SCRIM);
            cr.set_fill_rule(gtk4::cairo::FillRule::EvenOdd);
            cr.rectangle(ox, oy, iw, ih);
            cr.rectangle(x, y, w, h);
            let _ = cr.fill();
            // Thirds, then the border and corner handles.
            super::look::cairo_color(cr, crate::palette::CROP_GRID);
            cr.set_line_width(1.0);
            for i in 1..3 {
                let f = f64::from(i) / 3.0;
                cr.move_to(x + w * f, y);
                cr.line_to(x + w * f, y + h);
                cr.move_to(x, y + h * f);
                cr.line_to(x + w, y + h * f);
            }
            let _ = cr.stroke();
            super::look::cairo_color(cr, crate::palette::CROP_EDGE);
            cr.set_line_width(2.0);
            cr.rectangle(x, y, w, h);
            let _ = cr.stroke();
            for (cx, cy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                cr.rectangle(cx - 5.0, cy - 5.0, 10.0, 10.0);
                let _ = cr.fill();
            }
        });
    }

    let drag = gtk4::GestureDrag::new();
    {
        let crop = crop.clone();
        let canvas = canvas.clone();
        drag.connect_drag_begin(move |_, sx, sy| {
            let (ox, oy, scale) = placement(
                crop.dims,
                f64::from(canvas.width()),
                f64::from(canvas.height()),
            );
            let r = crop.rect.get();
            let (x, y) = (ox + f64::from(r.x) * scale, oy + f64::from(r.y) * scale);
            let (w, h) = (f64::from(r.width) * scale, f64::from(r.height) * scale);
            let corners = [
                (Corner::TopLeft, x, y),
                (Corner::TopRight, x + w, y),
                (Corner::BottomLeft, x, y + h),
                (Corner::BottomRight, x + w, y + h),
            ];
            let near = corners
                .into_iter()
                .find(|(_, cx, cy)| (sx - cx).abs() <= HANDLE && (sy - cy).abs() <= HANDLE);
            crop.drag.set(if let Some((corner, _, _)) = near {
                Some(Drag::Resize(corner, r))
            } else if (x..=x + w).contains(&sx) && (y..=y + h).contains(&sy) {
                Some(Drag::Move(r))
            } else {
                None
            });
        });
    }
    {
        let crop = crop.clone();
        let canvas = canvas.clone();
        let update_size = update_size.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let (_, _, scale) = placement(
                crop.dims,
                f64::from(canvas.width()),
                f64::from(canvas.height()),
            );
            if scale <= 0.0 {
                return;
            }
            let (dx, dy) = (dx / scale, dy / scale);
            let rect = match crop.drag.get() {
                Some(Drag::Move(start)) => start.moved(dx, dy, crop.dims),
                Some(Drag::Resize(corner, start)) => {
                    start.resized(corner, dx, dy, crop.dims, crop.aspect.get())
                }
                None => return,
            };
            crop.rect.set(rect);
            update_size();
            canvas.queue_draw();
        });
    }
    {
        let crop = crop.clone();
        drag.connect_drag_end(move |_, _, _| crop.drag.set(None));
    }
    canvas.add_controller(drag);

    {
        let crop = crop.clone();
        let canvas = canvas.clone();
        let update_size = update_size.clone();
        wide.connect_toggled(move |button| {
            let aspect = button.is_active().then_some((16, 9));
            crop.aspect.set(aspect);
            if aspect.is_some() {
                crop.rect.set(CropRect::centered(crop.dims, aspect));
            }
            update_size();
            canvas.queue_draw();
        });
    }

    // The side panels make way while cropping and come back afterwards.
    let panels = window.hide_side_panels();
    let close: Rc<dyn Fn()> = {
        let window = window.downgrade();
        let root = root.clone();
        let viewer = viewer.clone();
        Rc::new(move || {
            if root.parent().is_none() {
                return;
            }
            viewer.imp().overlay.remove_overlay(&root);
            if let Some(window) = window.upgrade() {
                window.set_crop_session(None);
                window.set_mode_header(None);
                window.restore_side_panels(panels);
            }
        })
    };
    let save_crop: Rc<dyn Fn()> = {
        let window = window.downgrade();
        let crop = crop.clone();
        let destination = destination.clone();
        let close = close.clone();
        Rc::new(move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let changed = std::fs::metadata(&crop.source)
                .map(|m| crate::fingerprint::file_stamp(&m))
                .ok()
                != Some(crop.stamp);
            if changed {
                window.add_toast(libadwaita::Toast::new(
                    "Not saved: the image changed since cropping started",
                ));
                return;
            }
            let folder = destination.borrow().clone();
            let config = CropConfig {
                source: crop.source.clone(),
                destination_dir: folder.clone(),
                rectangle: crop.rect.get(),
            };
            {
                let state = window.app_state();
                let mut st = state.borrow_mut();
                let mut export = st.settings.webp_export.clone();
                export.dir =
                    (folder != crate::config::settings::wallpapers_dir()).then_some(folder);
                st.settings.set_webp_export(export);
            }
            close();
            let operation = window.app_state().borrow().ops.add("Saving cropped copy…");
            let source = config.source.clone();
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(crate::export::crop_copy(&config));
            });
            let weak = window.downgrade();
            glib::MainContext::default().spawn_local(async move {
                let Ok(result) = rx.recv().await else {
                    operation.fail("Crop worker stopped");
                    return;
                };
                let Some(window) = weak.upgrade() else { return };
                match result {
                    Ok(result) => {
                        operation.complete();
                        window.add_edit_copies(
                            crate::buckets::EditKind::Cropped,
                            vec![(source.clone(), result.output.clone())],
                        );
                        let toast = libadwaita::Toast::new(&format!(
                            "Saved {}",
                            result
                                .output
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                        ));
                        toast.set_button_label(Some("Show"));
                        let weak = window.downgrade();
                        toast.connect_button_clicked(move |_| {
                            if let Some(window) = weak.upgrade() {
                                window.show_created_copy(result.output.clone());
                            }
                        });
                        window.add_toast(toast);
                    }
                    Err(error) => {
                        operation.fail(error.to_string());
                        window.add_toast(libadwaita::Toast::new(&format!("Crop failed: {error}")));
                    }
                }
            });
        })
    };
    {
        let close = close.clone();
        cancel.connect_clicked(move |_| close());
    }
    {
        let save_crop = save_crop.clone();
        save.connect_clicked(move |_| save_crop());
    }
    {
        let window = window.downgrade();
        let destination = destination.clone();
        folder.connect_clicked(move |button| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let chooser = gtk4::FileDialog::new();
            chooser.set_initial_folder(Some(&gio::File::for_path(&*destination.borrow())));
            let destination = destination.clone();
            let button = button.clone();
            let folder_text = folder_text.clone();
            chooser.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
                if let Some(path) = result.ok().and_then(|file| file.path()) {
                    folder_text.set_text(&folder_label(&path));
                    button.set_tooltip_text(Some(&format!("Destination: {}", path.display())));
                    *destination.borrow_mut() = path;
                }
            });
        });
    }
    viewer.imp().overlay.add_overlay(&root);
    window.set_mode_header(Some(&controls));
    root.grab_focus();
    window.set_crop_session(Some(CropSession {
        crop,
        canvas,
        update_size,
        save: save_crop,
        close,
    }));
}

/// A crop in progress, reachable from tests and the window.
#[derive(Clone)]
pub struct CropSession {
    crop: Rc<Crop>,
    canvas: gtk4::DrawingArea,
    update_size: Rc<dyn Fn()>,
    save: Rc<dyn Fn()>,
    close: Rc<dyn Fn()>,
}

impl CropSession {
    pub fn rect(&self) -> CropRect {
        self.crop.rect.get()
    }

    pub fn set_rect(&self, rect: CropRect) {
        if rect.fits(self.crop.dims) {
            self.crop.rect.set(rect);
            (self.update_size)();
            self.canvas.queue_draw();
        }
    }

    pub fn save(&self) {
        (self.save)();
    }

    pub fn close(&self) {
        (self.close)();
    }
}
