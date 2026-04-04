use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::ui::metadata_chip::MetadataChip;
use crate::ui::window::AppState;

// ---------------------------------------------------------------------------
// ViewerPane
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct ViewerPane {
        pub overlay: gtk4::Overlay,
        pub picture: gtk4::Picture,
        pub metadata_chip: MetadataChip,
        pub spinner: gtk4::Spinner,
        pub zoom: Cell<f64>,
        pub metadata_visible: Cell<bool>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
    }

    impl Default for ViewerPane {
        fn default() -> Self {
            let picture = gtk4::Picture::new();
            picture.set_content_fit(gtk4::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_can_shrink(true);

            let spinner = gtk4::Spinner::new();
            spinner.set_halign(gtk4::Align::Center);
            spinner.set_valign(gtk4::Align::Center);
            spinner.set_size_request(48, 48);
            spinner.set_visible(false);

            let metadata_chip = MetadataChip::new();
            metadata_chip.set_halign(gtk4::Align::End);
            metadata_chip.set_valign(gtk4::Align::End);
            metadata_chip.set_margin_end(16);
            metadata_chip.set_margin_bottom(16);

            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(&picture));
            overlay.add_overlay(&metadata_chip);
            overlay.add_overlay(&spinner);

            Self {
                overlay,
                picture,
                metadata_chip,
                spinner,
                zoom: Cell::new(1.0),
                metadata_visible: Cell::new(true),
                state: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ViewerPane {
        const NAME: &'static str = "SharprViewerPane";
        type Type = super::ViewerPane;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for ViewerPane {
        fn dispose(&self) {
            self.overlay.unparent();
        }
    }

    impl WidgetImpl for ViewerPane {}
}

glib::wrapper! {
    pub struct ViewerPane(ObjectSubclass<imp::ViewerPane>)
        @extends gtk4::Widget;
}

impl ViewerPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        let widget: Self = glib::Object::new();
        *widget.imp().state.borrow_mut() = Some(state);
        widget.build_ui();
        widget
    }

    fn build_ui(&self) {
        let imp = self.imp();
        imp.overlay.set_parent(self);

        // -----------------------------------------------------------------------
        // Keyboard shortcuts
        // -----------------------------------------------------------------------
        let shortcuts = gtk4::ShortcutController::new();
        shortcuts.set_scope(gtk4::ShortcutScope::Managed);

        // Ctrl+0 — reset zoom
        let w = self.downgrade();
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Control>0").unwrap()),
            Some(gtk4::CallbackAction::new(move |_, _| {
                if let Some(viewer) = w.upgrade() {
                    viewer.reset_zoom();
                }
                glib::Propagation::Stop
            })),
        ));

        // Alt+Return — toggle metadata overlay
        let w = self.downgrade();
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Alt>Return").unwrap()),
            Some(gtk4::CallbackAction::new(move |_, _| {
                if let Some(viewer) = w.upgrade() {
                    viewer.toggle_metadata();
                }
                glib::Propagation::Stop
            })),
        ));

        self.add_controller(shortcuts);

        // -----------------------------------------------------------------------
        // Ctrl+Scroll → zoom
        // -----------------------------------------------------------------------
        let scroll = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL
                | gtk4::EventControllerScrollFlags::KINETIC,
        );
        let w = self.downgrade();
        scroll.connect_scroll(move |ctrl, _dx, dy| {
            if ctrl
                .current_event_state()
                .contains(gdk4::ModifierType::CONTROL_MASK)
            {
                if let Some(viewer) = w.upgrade() {
                    let factor = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
                    viewer.apply_zoom(factor);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.add_controller(scroll);
    }

    // -----------------------------------------------------------------------
    // Image loading (async via background thread + idle callback)
    // -----------------------------------------------------------------------

    /// Clear the viewer (called when the folder changes).
    pub fn clear(&self) {
        let imp = self.imp();
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();
        imp.spinner.stop();
        imp.spinner.set_visible(false);
        self.reset_zoom();
    }

    /// Load and display a full-resolution image from `path`.
    ///
    /// The image is decoded on a background thread using the `image` crate.
    /// Raw RGBA bytes are sent back to the main thread via a one-shot channel,
    /// where a `gdk4::MemoryTexture` is constructed and set on the `GtkPicture`.
    pub fn load_image(&self, path: PathBuf) {
        let imp = self.imp();
        imp.spinner.start();
        imp.spinner.set_visible(true);
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();

        // Channel for decoded pixel data: (rgba_bytes, width, height)
        let (pixel_tx, pixel_rx) =
            async_channel::bounded::<Option<(Vec<u8>, u32, u32)>>(1);

        // Spawn background decode thread.
        let path_decode = path.clone();
        std::thread::spawn(move || {
            let result = decode_image_rgba(&path_decode);
            let _ = pixel_tx.send_blocking(result);
        });

        // Spawn background metadata thread.
        // Uses the same async-channel + spawn_local pattern so no Send requirement
        // is placed on the widget reference.
        let path_meta = path.clone();
        let (meta_tx, meta_rx) =
            async_channel::bounded::<crate::metadata::ImageMetadata>(1);
        std::thread::spawn(move || {
            let metadata = crate::metadata::ImageMetadata::load(&path_meta);
            let _ = meta_tx.send_blocking(metadata);
        });
        let widget_weak_meta = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(metadata) = meta_rx.recv().await {
                if let Some(viewer) = widget_weak_meta.upgrade() {
                    viewer.imp().metadata_chip.update(&metadata);
                }
            }
        });

        // Receive decoded pixels on the main thread.
        let widget_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(maybe_pixels) = pixel_rx.recv().await else {
                return;
            };
            let Some(viewer) = widget_weak.upgrade() else {
                return;
            };
            let imp = viewer.imp();
            imp.spinner.stop();
            imp.spinner.set_visible(false);

            match maybe_pixels {
                Some((bytes, w, h)) => {
                    let gbytes = glib::Bytes::from_owned(bytes);
                    let texture = gdk4::MemoryTexture::new(
                        w as i32,
                        h as i32,
                        gdk4::MemoryFormat::R8g8b8a8,
                        &gbytes,
                        (w * 4) as usize,
                    );
                    imp.picture
                        .set_paintable(Some(texture.upcast_ref::<gdk4::Paintable>()));
                    viewer.reset_zoom();
                }
                None => {
                    // Decode failed — clear to blank.
                    imp.picture.set_paintable(None::<&gdk4::Paintable>);
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // Zoom
    // -----------------------------------------------------------------------

    fn apply_zoom(&self, factor: f64) {
        let imp = self.imp();
        let new_zoom = (imp.zoom.get() * factor).clamp(0.05, 20.0);
        imp.zoom.set(new_zoom);
        // MVP zoom: adjust the picture's requested size; GtkPicture scales within.
        // Phase 4 will replace this with a proper transform-based zoom widget.
        let base = 800_i32;
        let scaled = (base as f64 * new_zoom) as i32;
        imp.picture.set_size_request(scaled, -1);
    }

    fn reset_zoom(&self) {
        let imp = self.imp();
        imp.zoom.set(1.0);
        imp.picture.set_size_request(-1, -1);
    }

    // -----------------------------------------------------------------------
    // Metadata overlay
    // -----------------------------------------------------------------------

    fn toggle_metadata(&self) {
        let imp = self.imp();
        let visible = !imp.metadata_visible.get();
        imp.metadata_visible.set(visible);
        imp.metadata_chip.set_visible(visible);
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust image decode (runs on background thread — no GTK calls)
// ---------------------------------------------------------------------------

fn decode_image_rgba(path: &PathBuf) -> Option<(Vec<u8>, u32, u32)> {
    use image::ImageReader;
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path).ok()?;
    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}
