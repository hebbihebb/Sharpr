//! Crop Copy in the viewer at full size (`ROADMAP.md` Phase 6): the image
//! fills the viewer with a selection to move and resize, 16:9 or free, and
//! saves a new copy in the source format. It opens on the viewer's zoom and
//! position, and scrolling zooms at the pointer while the selection stays on
//! the same pixels. The side panels make way while cropping; F11 makes it
//! fullscreen. Originals are only read.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::export::{Corner, CropConfig, CropRect};
use crate::ui::crop_view::{self, CropView};
use crate::ui::window::SharprWindow;

/// The longest edge of a preview decoded on a worker; zooming in past what it
/// holds loads the full image.
const PREVIEW_EDGE: u32 = 2560;
/// One scroll step's zoom, as in the viewer.
const ZOOM_STEP: f64 = 1.1;
/// How close to a corner, in screen pixels, a drag resizes.
const HANDLE: f64 = 18.0;

#[derive(Clone, Copy)]
enum Drag {
    Move(CropRect),
    Resize(Corner, CropRect),
    /// Dragging the image itself, from this view.
    Pan(CropView),
}

/// The viewer's scale (screen pixels per image pixel) and centre (fractions
/// of the image).
type ViewerView = (f64, (f64, f64));

struct Crop {
    source: PathBuf,
    /// The source's size and modification time when cropping started.
    stamp: (u64, i64),
    dims: (u32, u32),
    /// The selection, in image pixels, so it stays on the same pixels
    /// whatever the zoom and pan do.
    rect: Cell<CropRect>,
    aspect: Cell<Option<(u32, u32)>>,
    drag: Cell<Option<Drag>>,
    view: Cell<CropView>,
    /// The viewer's view to open on. It holds, as a scale and a centre,
    /// through the canvas settling (the side panels slide away as crop
    /// opens) until the user first zooms or pans; then the view is theirs.
    opening: Cell<Option<ViewerView>>,
    /// What the canvas shows: the viewer's picture or a preview, then the
    /// full image once zooming needs it.
    image: RefCell<gdk4::Paintable>,
    full_requested: Cell<bool>,
    /// Crop mode has been left; late results are dropped.
    closed: Cell<bool>,
    /// The pointer on the canvas, where scrolling zooms.
    pointer: Cell<Option<(f64, f64)>>,
}

impl Crop {
    /// The view for a canvas of `size`: the viewer's view while it is held,
    /// otherwise the current one kept valid for the canvas.
    fn view(&self, size: (f64, f64)) -> CropView {
        if size.0 <= 0.0 || size.1 <= 0.0 {
            return self.view.get();
        }
        let view = match self.opening.get() {
            Some((scale, fraction)) => CropView::from_scale(scale, fraction, self.dims, size),
            None => self.view.get().clamped(self.dims, size),
        };
        self.view.set(view);
        view
    }

    /// The user zooms or pans: stop holding the viewer's view.
    fn release_opening(&self, size: (f64, f64)) {
        if self.opening.get().is_some() {
            let view = self.view(size);
            self.opening.set(None);
            self.view.set(view);
        }
    }

    /// The selection on a canvas of `size`: x, y, width, height.
    fn selection(&self, size: (f64, f64)) -> (f64, f64, f64, f64) {
        let (ox, oy, scale) = self.view(size).placement(self.dims, size);
        let r = self.rect.get();
        (
            ox + f64::from(r.x) * scale,
            oy + f64::from(r.y) * scale,
            f64::from(r.width) * scale,
            f64::from(r.height) * scale,
        )
    }
}

fn canvas_size(canvas: &crate::ui::guide_layer::GuideLayer) -> (f64, f64) {
    (f64::from(canvas.width()), f64::from(canvas.height()))
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
    // Open on the viewer's zoom and position when it shows this image.
    let viewer_view = shown.as_ref().and_then(|_| viewer.view_state());
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
                    open(&window, source, dims, stamp, frame, viewer_view, paintable);
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
    viewer_view: Option<ViewerView>,
    paintable: gdk4::Paintable,
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
        view: Cell::new(CropView::fit(dims)),
        opening: Cell::new(viewer_view),
        image: RefCell::new(paintable),
        full_requested: Cell::new(false),
        closed: Cell::new(false),
        pointer: Cell::new(None),
    });

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("background");
    root.set_focusable(true);
    let stage = gtk4::Overlay::new();
    stage.set_vexpand(true);
    // Look prototype: a neutral canvas in both themes (I14).
    stage.add_css_class("sharpr-canvas");
    // The canvas draws the image as well as the selection, and covers the
    // whole viewer, so the image starts exactly where the viewer had it.
    let canvas = crate::ui::guide_layer::GuideLayer::default();
    stage.set_child(Some(&canvas));
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
    // Look prototype: the keys that work here, in one quiet line (D26),
    // over the canvas so the canvas keeps the viewer's size.
    let hints = gtk4::Label::new(None);
    hints.set_markup(
        "scroll to zoom · drag the picture to pan · drag the box to move · drag a corner to resize   <b>Enter</b> save   <b>Esc</b> cancel",
    );
    hints.add_css_class("sharpr-keys");
    hints.add_css_class("sharpr-keys-osd");
    hints.set_wrap(true);
    hints.set_halign(gtk4::Align::Center);
    hints.set_valign(gtk4::Align::End);
    hints.set_margin_bottom(12);
    hints.set_margin_start(12);
    hints.set_margin_end(12);
    hints.set_can_target(false);
    stage.add_overlay(&hints);
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

    // Zooming past what the picture holds loads the full image once, off the
    // GTK thread; it is asked for from the draw, which sees every view change.
    let want_full: Rc<dyn Fn()> = {
        let crop = crop.clone();
        let canvas = canvas.downgrade();
        let window = window.downgrade();
        Rc::new(move || {
            if crop.full_requested.get() || crop.closed.get() {
                return;
            }
            let (Some(canvas), Some(window)) = (canvas.upgrade(), window.upgrade()) else {
                return;
            };
            let size = canvas_size(&canvas);
            if size.0 <= 0.0 || size.1 <= 0.0 {
                return;
            }
            let scale = crop.view(size).scale(crop.dims, size);
            let held = crop.image.borrow().intrinsic_width().max(1) as u32;
            if crop_view::needs_full_resolution(
                held,
                crop.dims.0,
                scale,
                f64::from(canvas.scale_factor().max(1)),
            ) {
                load_full(&window, &crop, &canvas);
            }
        })
    };

    {
        let crop = crop.clone();
        let weak = canvas.downgrade();
        let want_full = want_full.clone();
        canvas.set_draw(move |snapshot| {
            use crate::ui::guide_layer as layer;
            let Some(canvas) = weak.upgrade() else { return };
            let size = canvas_size(&canvas);
            let (ox, oy, scale) = crop.view(size).placement(crop.dims, size);
            let image = (
                ox,
                oy,
                f64::from(crop.dims.0) * scale,
                f64::from(crop.dims.1) * scale,
            );
            let (x, y, w, h) = crop.selection(size);
            // Nothing is drawn outside the canvas.
            snapshot.push_clip(&layer::graphene((0.0, 0.0, size.0, size.1)));
            draw_image(snapshot, &crop.image.borrow(), image);
            // Shade the visible image outside the selection.
            layer::dim(
                snapshot,
                intersection(image, (0.0, 0.0, size.0, size.1)),
                (x, y, w, h),
            );
            // Thirds, then the border and corner handles.
            for i in 1..3 {
                let f = f64::from(i) / 3.0;
                layer::fill(
                    snapshot,
                    crate::palette::CROP_GRID,
                    (x + w * f - 0.5, y, 1.0, h),
                );
                layer::fill(
                    snapshot,
                    crate::palette::CROP_GRID,
                    (x, y + h * f - 0.5, w, 1.0),
                );
            }
            layer::outline(snapshot, crate::palette::CROP_EDGE, (x, y, w, h), 2.0);
            layer::handles(snapshot, (x, y, w, h));
            snapshot.pop();
            want_full();
        });
    }

    // Drag a corner to resize, inside the box to move it, elsewhere to pan
    // the picture; the middle button pans from anywhere.
    let drag = gtk4::GestureDrag::new();
    {
        let crop = crop.clone();
        let canvas = canvas.clone();
        drag.connect_drag_begin(move |_, sx, sy| {
            let size = canvas_size(&canvas);
            let (x, y, w, h) = crop.selection(size);
            let r = crop.rect.get();
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
                crop.release_opening(size);
                Some(Drag::Pan(crop.view(size)))
            });
        });
    }
    let drag_update: Rc<dyn Fn(f64, f64)> = {
        let crop = crop.clone();
        let canvas = canvas.clone();
        let update_size = update_size.clone();
        Rc::new(move |dx, dy| {
            let size = canvas_size(&canvas);
            let scale = crop.view(size).scale(crop.dims, size);
            if scale <= 0.0 {
                return;
            }
            let (ix, iy) = (dx / scale, dy / scale);
            let rect = match crop.drag.get() {
                Some(Drag::Move(start)) => start.moved(ix, iy, crop.dims),
                Some(Drag::Resize(corner, start)) => {
                    start.resized(corner, ix, iy, crop.dims, crop.aspect.get())
                }
                Some(Drag::Pan(start)) => {
                    crop.view.set(start.panned(dx, dy, crop.dims, size));
                    canvas.queue_draw();
                    return;
                }
                None => return,
            };
            crop.rect.set(rect);
            update_size();
            canvas.queue_draw();
        })
    };
    {
        let drag_update = drag_update.clone();
        drag.connect_drag_update(move |_, dx, dy| drag_update(dx, dy));
    }
    {
        let crop = crop.clone();
        drag.connect_drag_end(move |_, _, _| crop.drag.set(None));
    }
    canvas.add_controller(drag);

    let pan = gtk4::GestureDrag::new();
    pan.set_button(gdk4::BUTTON_MIDDLE);
    {
        let crop = crop.clone();
        let canvas = canvas.clone();
        pan.connect_drag_begin(move |_, _, _| {
            let size = canvas_size(&canvas);
            crop.release_opening(size);
            crop.drag.set(Some(Drag::Pan(crop.view(size))));
        });
    }
    pan.connect_drag_update(move |_, dx, dy| drag_update(dx, dy));
    {
        let crop = crop.clone();
        pan.connect_drag_end(move |_, _, _| crop.drag.set(None));
    }
    canvas.add_controller(pan);

    // Scrolling zooms at the pointer (the viewer steps by the same amount).
    let motion = gtk4::EventControllerMotion::new();
    {
        let crop = crop.clone();
        motion.connect_motion(move |_, x, y| crop.pointer.set(Some((x, y))));
    }
    {
        let crop = crop.clone();
        motion.connect_leave(move |_| crop.pointer.set(None));
    }
    canvas.add_controller(motion);
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    {
        let crop = crop.clone();
        let canvas = canvas.clone();
        scroll.connect_scroll(move |_, _, dy| {
            let size = canvas_size(&canvas);
            let at = crop.pointer.get().unwrap_or((size.0 / 2.0, size.1 / 2.0));
            let factor = if dy < 0.0 { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
            zoom_at(&crop, &canvas, factor, at);
            glib::Propagation::Stop
        });
    }
    canvas.add_controller(scroll);

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
        let crop = crop.clone();
        Rc::new(move || {
            if root.parent().is_none() {
                return;
            }
            crop.closed.set(true);
            viewer.set_crop_open(false);
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
    viewer.set_crop_open(true);
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

/// Zooms by `factor` with the image pixel under `at` staying under it.
fn zoom_at(crop: &Crop, canvas: &crate::ui::guide_layer::GuideLayer, factor: f64, at: (f64, f64)) {
    let size = canvas_size(canvas);
    crop.release_opening(size);
    crop.view
        .set(crop.view(size).zoomed_at(factor, at, crop.dims, size));
    canvas.queue_draw();
}

/// The part of `a` inside `b`, or an empty rectangle.
fn intersection(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

/// Draws `paintable` filling `rect`, smoothing when it shrinks it.
fn draw_image(snapshot: &gtk4::Snapshot, paintable: &gdk4::Paintable, rect: (f64, f64, f64, f64)) {
    use crate::ui::guide_layer as layer;
    if let Some(texture) = paintable.downcast_ref::<gdk4::Texture>() {
        let filter = if f64::from(texture.width()) > rect.2 {
            gtk4::gsk::ScalingFilter::Trilinear
        } else {
            gtk4::gsk::ScalingFilter::Linear
        };
        snapshot.append_scaled_texture(texture, filter, &layer::graphene(rect));
    } else {
        snapshot.save();
        snapshot.translate(&gtk4::graphene::Point::new(rect.0 as f32, rect.1 as f32));
        paintable.snapshot(snapshot, rect.2, rect.3);
        snapshot.restore();
    }
}

/// Decodes the whole image on a worker and swaps it in for the preview, for
/// zooming in. One request per crop; a failure keeps the preview.
fn load_full(window: &SharprWindow, crop: &Rc<Crop>, canvas: &crate::ui::guide_layer::GuideLayer) {
    crop.full_requested.set(true);
    let (tx, rx) = async_channel::bounded(1);
    let source = crop.source.clone();
    std::thread::spawn(move || {
        let result = crate::export::decode_oriented(&source)
            .map(|image| {
                let rgba = image.into_rgba8();
                (rgba.width(), rgba.height(), rgba.into_raw())
            })
            .map_err(|e| e.to_string());
        let _ = tx.send_blocking(result);
    });
    let crop = crop.clone();
    let canvas = canvas.downgrade();
    let window = window.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = rx.recv().await else { return };
        if crop.closed.get() {
            return;
        }
        match result {
            Ok((w, h, rgba)) => {
                let texture = gdk4::MemoryTexture::new(
                    w as i32,
                    h as i32,
                    gdk4::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from_owned(rgba),
                    w as usize * 4,
                );
                *crop.image.borrow_mut() = texture.upcast();
                if let Some(canvas) = canvas.upgrade() {
                    canvas.queue_draw();
                }
            }
            Err(error) => {
                if let Some(window) = window.upgrade() {
                    window.add_toast(libadwaita::Toast::new(&format!(
                        "Could not load the full image to zoom: {error}"
                    )));
                }
            }
        }
    });
}

/// A crop in progress, reachable from tests and the window.
#[derive(Clone)]
pub struct CropSession {
    crop: Rc<Crop>,
    canvas: crate::ui::guide_layer::GuideLayer,
    update_size: Rc<dyn Fn()>,
    save: Rc<dyn Fn()>,
    close: Rc<dyn Fn()>,
}

impl CropSession {
    pub fn rect(&self) -> CropRect {
        self.crop.rect.get()
    }

    /// The zoom (times fit) and the image point at the canvas centre.
    pub fn view(&self) -> CropView {
        self.crop.view(canvas_size(&self.canvas))
    }

    /// Zooms by `factor` at the canvas point `at`, as scrolling does.
    pub fn zoom_at(&self, factor: f64, at: (f64, f64)) {
        zoom_at(&self.crop, &self.canvas, factor, at);
    }

    /// Where the image's top-left corner lands on the canvas, and the scale.
    pub fn image_on_canvas(&self) -> (f64, f64, f64) {
        self.view()
            .placement(self.crop.dims, canvas_size(&self.canvas))
    }

    /// The selection on the canvas, in canvas pixels: x, y, width, height.
    pub fn selection_on_canvas(&self) -> (f64, f64, f64, f64) {
        self.crop.selection(canvas_size(&self.canvas))
    }

    /// The canvas, to place it against the viewer's overlay.
    pub fn canvas(&self) -> gtk4::Widget {
        self.canvas.clone().upcast()
    }

    /// The width in pixels of the picture now drawn.
    pub fn picture_width(&self) -> i32 {
        self.crop.image.borrow().intrinsic_width()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn crop(opening: Option<ViewerView>) -> Crop {
        let texture = gdk4::MemoryTexture::new(
            1,
            1,
            gdk4::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(vec![0u8; 4]),
            4,
        );
        Crop {
            source: PathBuf::new(),
            stamp: (0, 0),
            dims: (4000, 2000),
            rect: Cell::new(CropRect::centered((4000, 2000), Some((16, 9)))),
            aspect: Cell::new(Some((16, 9))),
            drag: Cell::new(None),
            view: Cell::new(CropView::fit((4000, 2000))),
            opening: Cell::new(opening),
            image: RefCell::new(texture.upcast()),
            full_requested: Cell::new(false),
            closed: Cell::new(false),
            pointer: Cell::new(None),
        }
    }

    fn scale(crop: &Crop, size: (f64, f64)) -> f64 {
        crop.view(size).scale(crop.dims, size)
    }

    #[test]
    fn the_viewer_s_scale_holds_while_the_canvas_settles() {
        // The side panels slide away, so the first draws see a narrower
        // canvas than the last: the scale must not follow it.
        let crop = crop(Some((1.0, (0.5, 0.5))));
        for width in [900.0, 1000.0, 1300.0, 1700.0] {
            assert!((scale(&crop, (width, 600.0)) - 1.0).abs() < 1e-9, "{width}");
        }
    }

    #[test]
    fn once_the_user_zooms_the_view_is_theirs() {
        let crop = crop(Some((1.0, (0.5, 0.5))));
        let size = (1000.0, 600.0);
        let held = scale(&crop, size);
        crop.release_opening(size);
        crop.view.set(
            crop.view(size)
                .zoomed_at(2.0, (500.0, 300.0), crop.dims, size),
        );
        let zoomed = scale(&crop, size);
        assert!((zoomed - held * 2.0).abs() < 1e-9);
        // A later resize now keeps the zoom relative to fit.
        let wider = (2000.0, 600.0);
        let fit = crop_view::fit_scale(crop.dims, wider);
        assert!((scale(&crop, wider) - fit * crop.view.get().zoom).abs() < 1e-9);
    }
}
