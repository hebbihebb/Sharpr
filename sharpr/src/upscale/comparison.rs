use std::cell::Cell;
use std::path::PathBuf;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    None,
    Divider,
    Pan,
}

// ---------------------------------------------------------------------------
// BeforeAfterViewer
//
// Side-by-side comparison widget. Left of the draggable divider shows the
// "before" image; right shows "after". Rendered via GtkWidget::snapshot() so
// the GPU compositor handles everything — no Cairo pixel-loops required.
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct BeforeAfterViewer {
        /// Divider position as a fraction of widget width in [0, 1].
        pub divider: Cell<f64>,
        pub zoom: Cell<f64>,
        pub pan_x: Cell<f64>,
        pub pan_y: Cell<f64>,
        pub before_texture: std::cell::RefCell<Option<gdk4::Texture>>,
        pub after_texture: std::cell::RefCell<Option<gdk4::Texture>>,
        pub pointer_pos: Cell<(f64, f64)>,
        pub(super) drag_mode: Cell<DragMode>,
        pub pan_origin: Cell<Option<(f64, f64)>>,
        pub load_gen: Cell<u64>,
    }

    impl Default for BeforeAfterViewer {
        fn default() -> Self {
            Self {
                divider: Cell::new(0.5),
                zoom: Cell::new(1.0),
                pan_x: Cell::new(0.0),
                pan_y: Cell::new(0.0),
                before_texture: std::cell::RefCell::new(None),
                after_texture: std::cell::RefCell::new(None),
                pointer_pos: Cell::new((0.0, 0.0)),
                drag_mode: Cell::new(DragMode::None),
                pan_origin: Cell::new(None),
                load_gen: Cell::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BeforeAfterViewer {
        const NAME: &'static str = "SharprBeforeAfterViewer";
        type Type = super::BeforeAfterViewer;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for BeforeAfterViewer {}

    impl WidgetImpl for BeforeAfterViewer {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let widget = self.obj();
            let w = widget.width() as f32;
            let h = widget.height() as f32;
            if w <= 0.0 || h <= 0.0 {
                return;
            }

            let divider_x = (self.divider.get() as f32 * w).clamp(0.0, w);
            let full = gtk4::graphene::Rect::new(0.0, 0.0, w, h);
            let zoom = self.zoom.get() as f32;
            let pan_x = self.pan_x.get() as f32;
            let pan_y = self.pan_y.get() as f32;
            let img_w = w * zoom;
            let img_h = h * zoom;
            let origin_x = (w - img_w) / 2.0 + pan_x;
            let origin_y = (h - img_h) / 2.0 + pan_y;
            let img_rect = gtk4::graphene::Rect::new(origin_x, origin_y, img_w, img_h);

            // Opaque dark background so RGBA images with transparency don't
            // show a GTK checkerboard pattern.
            snapshot.append_color(&gdk4::RGBA::new(0.12, 0.12, 0.12, 1.0), &full);

            // Before — left of divider.
            if let Some(ref tex) = *self.before_texture.borrow() {
                snapshot.push_clip(&gtk4::graphene::Rect::new(0.0, 0.0, divider_x, h));
                snapshot.append_texture(tex, &img_rect);
                snapshot.pop();
            }

            // After — right of divider.
            if let Some(ref tex) = *self.after_texture.borrow() {
                snapshot.push_clip(&gtk4::graphene::Rect::new(divider_x, 0.0, w - divider_x, h));
                snapshot.append_texture(tex, &img_rect);
                snapshot.pop();
            }

            // Divider line.
            snapshot.append_color(
                &gdk4::RGBA::new(1.0, 1.0, 1.0, 0.85),
                &gtk4::graphene::Rect::new(divider_x - 1.0, 0.0, 2.0, h),
            );

            // Drag handle — small white square centred on the divider.
            let nub = 20.0_f32;
            snapshot.append_color(
                &gdk4::RGBA::new(1.0, 1.0, 1.0, 0.9),
                &gtk4::graphene::Rect::new(divider_x - nub / 2.0, h / 2.0 - nub / 2.0, nub, nub),
            );
        }
    }
}

glib::wrapper! {
    pub struct BeforeAfterViewer(ObjectSubclass<imp::BeforeAfterViewer>)
        @extends gtk4::Widget;
}

impl BeforeAfterViewer {
    pub fn new() -> Self {
        let widget: Self = glib::Object::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.setup_motion();
        widget.setup_drag();
        widget.setup_zoom();
        widget
    }

    /// Load before/after images from disk, decoding on background threads.
    /// Calls `on_ready` on the main thread once both textures are attached.
    pub fn load<F>(&self, before_path: PathBuf, after_path: PathBuf, on_ready: F)
    where
        F: FnOnce() + 'static,
    {
        let imp = self.imp();
        let load_gen = imp.load_gen.get().wrapping_add(1);
        imp.load_gen.set(load_gen);
        *imp.before_texture.borrow_mut() = None;
        *imp.after_texture.borrow_mut() = None;
        self.queue_draw();
        let on_ready = std::rc::Rc::new(std::cell::RefCell::new(Some(
            Box::new(on_ready) as Box<dyn FnOnce()>
        )));

        let (tx, rx) = async_channel::bounded::<(bool, Option<(Vec<u8>, i32, i32)>)>(2);

        let tx1 = tx.clone();
        let b = before_path.clone();
        std::thread::spawn(move || {
            let _ = tx1.send_blocking((false, decode_rgba(&b)));
        });
        std::thread::spawn(move || {
            let _ = tx.send_blocking((true, decode_rgba(&after_path)));
        });

        let widget_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let mut count = 0;
            while let Ok((is_after, pixels)) = rx.recv().await {
                let Some(w) = widget_weak.upgrade() else {
                    break;
                };
                let imp = w.imp();
                if imp.load_gen.get() != load_gen {
                    break;
                }
                if let Some((bytes, width, height)) = pixels {
                    let gbytes = glib::Bytes::from_owned(bytes);
                    let tex: gdk4::Texture = gdk4::MemoryTexture::new(
                        width,
                        height,
                        gdk4::MemoryFormat::R8g8b8a8,
                        &gbytes,
                        (width as usize) * 4,
                    )
                    .upcast();
                    if imp.load_gen.get() != load_gen {
                        break;
                    }
                    if is_after {
                        *imp.after_texture.borrow_mut() = Some(tex);
                    } else {
                        *imp.before_texture.borrow_mut() = Some(tex);
                    }
                }
                count += 1;
                if count == 2 {
                    w.queue_draw();
                    if let Some(cb) = on_ready.borrow_mut().take() {
                        cb();
                    }
                    break;
                }
            }
        });
    }

    pub fn clear(&self) {
        let imp = self.imp();
        imp.load_gen.set(imp.load_gen.get().wrapping_add(1));
        *imp.before_texture.borrow_mut() = None;
        *imp.after_texture.borrow_mut() = None;
        self.reset_zoom();
    }

    pub fn reset_zoom(&self) {
        let imp = self.imp();
        imp.zoom.set(1.0);
        imp.pan_x.set(0.0);
        imp.pan_y.set(0.0);
        imp.pan_origin.set(None);
        imp.drag_mode.set(DragMode::None);
        self.queue_draw();
    }

    fn setup_motion(&self) {
        let motion = gtk4::EventControllerMotion::new();

        let w = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().pointer_pos.set((x, y));
            }
        });

        self.add_controller(motion);
    }

    fn setup_drag(&self) {
        let drag = gtk4::GestureDrag::new();
        drag.set_button(gtk4::gdk::BUTTON_PRIMARY);

        let w = self.downgrade();
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            imp.pointer_pos.set((x, y));
            imp.pan_origin.set(Some((imp.pan_x.get(), imp.pan_y.get())));

            let width = viewer.width() as f64;
            if width <= 0.0 {
                imp.drag_mode.set(DragMode::None);
                gesture.set_state(gtk4::EventSequenceState::Denied);
                return;
            }

            let divider_x = imp.divider.get() * width;
            let handle_half_width = 12.0;
            if (x - divider_x).abs() <= handle_half_width {
                imp.drag_mode.set(DragMode::Divider);
            } else {
                imp.drag_mode.set(DragMode::Pan);
            }
        });

        let w = self.downgrade();
        drag.connect_drag_update(move |gesture, offset_x, offset_y| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            match imp.drag_mode.get() {
                DragMode::Divider => {
                    let width = viewer.width() as f64;
                    if width <= 0.0 {
                        return;
                    }
                    let (start_x, _) = gesture.start_point().unwrap_or((0.0, 0.0));
                    imp.divider
                        .set(((start_x + offset_x) / width).clamp(0.0, 1.0));
                    viewer.queue_draw();
                }
                DragMode::Pan => {
                    viewer.pan_drag(offset_x, offset_y);
                }
                DragMode::None => {}
            }
        });

        let w = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                let imp = viewer.imp();
                imp.drag_mode.set(DragMode::None);
                imp.pan_origin.set(None);
            }
        });

        self.add_controller(drag);

        let pan_drag = gtk4::GestureDrag::new();
        pan_drag.set_button(gtk4::gdk::BUTTON_MIDDLE);

        let w = self.downgrade();
        pan_drag.connect_drag_begin(move |_, _, _| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            imp.pan_origin.set(Some((imp.pan_x.get(), imp.pan_y.get())));
        });

        let w = self.downgrade();
        pan_drag.connect_drag_update(move |_, offset_x, offset_y| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            let Some((start_x, start_y)) = imp.pan_origin.get() else {
                return;
            };
            imp.pan_x.set(start_x + offset_x);
            imp.pan_y.set(start_y + offset_y);
            viewer.queue_draw();
        });

        let w = self.downgrade();
        pan_drag.connect_drag_end(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().pan_origin.set(None);
            }
        });

        self.add_controller(pan_drag);
    }

    fn setup_zoom(&self) {
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);

        let w = self.downgrade();
        scroll.connect_scroll(move |_, _dx, dy| {
            let Some(viewer) = w.upgrade() else {
                return glib::Propagation::Stop;
            };
            let factor = if dy < 0.0 { 1.1_f64 } else { 1.0 / 1.1 };
            viewer.apply_zoom(factor);
            glib::Propagation::Stop
        });

        self.add_controller(scroll);
    }

    fn pan_drag(&self, offset_x: f64, offset_y: f64) {
        let imp = self.imp();
        let Some((start_x, start_y)) = imp.pan_origin.get() else {
            return;
        };
        imp.pan_x.set(start_x + offset_x);
        imp.pan_y.set(start_y + offset_y);
        self.queue_draw();
    }

    fn apply_zoom(&self, factor: f64) {
        let imp = self.imp();
        let old_zoom = imp.zoom.get();
        let new_zoom = (old_zoom * factor).clamp(0.25, 8.0);
        if (new_zoom - old_zoom).abs() < f64::EPSILON {
            return;
        }

        let w = self.width() as f64;
        let h = self.height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return;
        }

        let (focus_x, focus_y) = imp.pointer_pos.get();
        let old_origin_x = (w - w * old_zoom) / 2.0 + imp.pan_x.get();
        let old_origin_y = (h - h * old_zoom) / 2.0 + imp.pan_y.get();
        let scale_ratio = new_zoom / old_zoom;
        let new_origin_x = focus_x - (focus_x - old_origin_x) * scale_ratio;
        let new_origin_y = focus_y - (focus_y - old_origin_y) * scale_ratio;
        let centered_origin_x = (w - w * new_zoom) / 2.0;
        let centered_origin_y = (h - h * new_zoom) / 2.0;

        imp.zoom.set(new_zoom);
        imp.pan_x.set(new_origin_x - centered_origin_x);
        imp.pan_y.set(new_origin_y - centered_origin_y);
        self.queue_draw();
    }
}

impl Default for BeforeAfterViewer {
    fn default() -> Self {
        Self::new()
    }
}

fn decode_rgba(path: &PathBuf) -> Option<(Vec<u8>, i32, i32)> {
    use image::ImageReader;
    use std::io::BufReader;
    let file = std::fs::File::open(path).ok()?;
    let img = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width() as i32, rgba.height() as i32);
    Some((rgba.into_raw(), w, h))
}
