use std::cell::Cell;
use std::path::PathBuf;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

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
        pub before_texture: std::cell::RefCell<Option<gdk4::Texture>>,
        pub after_texture: std::cell::RefCell<Option<gdk4::Texture>>,
        pub dragging: Cell<bool>,
        pub load_gen: Cell<u64>,
    }

    impl Default for BeforeAfterViewer {
        fn default() -> Self {
            Self {
                divider: Cell::new(0.5),
                before_texture: std::cell::RefCell::new(None),
                after_texture: std::cell::RefCell::new(None),
                dragging: Cell::new(false),
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

            // Opaque dark background so RGBA images with transparency don't
            // show a GTK checkerboard pattern.
            snapshot.append_color(&gdk4::RGBA::new(0.12, 0.12, 0.12, 1.0), &full);

            // Before — left of divider.
            if let Some(ref tex) = *self.before_texture.borrow() {
                snapshot.push_clip(&gtk4::graphene::Rect::new(0.0, 0.0, divider_x, h));
                snapshot.append_texture(tex, &full);
                snapshot.pop();
            }

            // After — right of divider.
            if let Some(ref tex) = *self.after_texture.borrow() {
                snapshot.push_clip(&gtk4::graphene::Rect::new(
                    divider_x, 0.0, w - divider_x, h,
                ));
                snapshot.append_texture(tex, &full);
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
                &gtk4::graphene::Rect::new(
                    divider_x - nub / 2.0,
                    h / 2.0 - nub / 2.0,
                    nub,
                    nub,
                ),
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
        widget.setup_drag();
        widget
    }

    /// Load before/after images from disk, decoding on background threads.
    pub fn load(&self, before_path: PathBuf, after_path: PathBuf) {
        let imp = self.imp();
        let load_gen = imp.load_gen.get().wrapping_add(1);
        imp.load_gen.set(load_gen);
        *imp.before_texture.borrow_mut() = None;
        *imp.after_texture.borrow_mut() = None;
        self.queue_draw();

        let (tx, rx) = async_channel::bounded::<(bool, Option<(Vec<u8>, i32, i32)>)>(2);

        let tx1 = tx.clone();
        let b = before_path.clone();
        std::thread::spawn(move || { let _ = tx1.send_blocking((false, decode_rgba(&b))); });
        std::thread::spawn(move || { let _ = tx.send_blocking((true, decode_rgba(&after_path))); });

        let widget_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let mut count = 0;
            while let Ok((is_after, pixels)) = rx.recv().await {
                let Some(w) = widget_weak.upgrade() else { break };
                let imp = w.imp();
                if imp.load_gen.get() != load_gen {
                    break;
                }
                if let Some((bytes, width, height)) = pixels {
                    let gbytes = glib::Bytes::from_owned(bytes);
                    let tex: gdk4::Texture = gdk4::MemoryTexture::new(
                        width, height, gdk4::MemoryFormat::R8g8b8a8,
                        &gbytes, (width as usize) * 4,
                    ).upcast();
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
        self.queue_draw();
    }

    fn setup_drag(&self) {
        let drag = gtk4::GestureDrag::new();
        drag.set_button(gtk4::gdk::BUTTON_PRIMARY);

        let w = self.downgrade();
        drag.connect_drag_begin(move |_, x, _| {
            let Some(viewer) = w.upgrade() else { return };
            let width = viewer.width() as f64;
            if width > 0.0 {
                viewer.imp().divider.set((x / width).clamp(0.0, 1.0));
                viewer.imp().dragging.set(true);
                viewer.queue_draw();
            }
        });

        let w = self.downgrade();
        drag.connect_drag_update(move |gesture, offset_x, _| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            if !imp.dragging.get() { return; }
            let width = viewer.width() as f64;
            if width <= 0.0 { return; }
            let (start_x, _) = gesture.start_point().unwrap_or((0.0, 0.0));
            imp.divider.set(((start_x + offset_x) / width).clamp(0.0, 1.0));
            viewer.queue_draw();
        });

        let w = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().dragging.set(false);
            }
        });

        self.add_controller(drag);
    }
}

impl Default for BeforeAfterViewer {
    fn default() -> Self { Self::new() }
}

fn decode_rgba(path: &PathBuf) -> Option<(Vec<u8>, i32, i32)> {
    use image::ImageReader;
    use std::io::BufReader;
    let file = std::fs::File::open(path).ok()?;
    let img = ImageReader::new(BufReader::new(file))
        .with_guessed_format().ok()?.decode().ok()?;
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width() as i32, rgba.height() as i32);
    Some((rgba.into_raw(), w, h))
}
