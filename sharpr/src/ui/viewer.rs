use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::ui::metadata_chip::MetadataChip;
use crate::ui::window::AppState;
use crate::upscale::BeforeAfterViewer;

// ---------------------------------------------------------------------------
// ViewerPane
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct ViewerPane {
        /// Root stack: "view" = normal viewer, "compare" = before/after widget.
        pub stack: gtk4::Stack,
        pub overlay: gtk4::Overlay,
        pub scrolled_window: gtk4::ScrolledWindow,
        pub picture: gtk4::Picture,
        pub metadata_chip: MetadataChip,
        pub spinner: gtk4::Spinner,
        /// OSD progress bar — shown during an upscale job, hidden otherwise.
        pub progress_bar: gtk4::ProgressBar,
        pub comparison: BeforeAfterViewer,
        /// Temp output path while the compare view is active.
        pub pending_output: std::cell::RefCell<Option<std::path::PathBuf>>,
        /// Commit/Discard buttons owned by the window header; stored here so
        /// async upscale callbacks can show/hide them without capturing clones.
        pub commit_btn: std::cell::RefCell<Option<gtk4::Button>>,
        pub discard_btn: std::cell::RefCell<Option<gtk4::Button>>,
        pub zoom: Cell<f64>,
        pub metadata_visible: Cell<bool>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub pointer_pos: Cell<(f64, f64)>,
        pub drag_origin: Cell<Option<(f64, f64)>>,
        pub drag_adjustments: Cell<(f64, f64)>,
        /// Monotonically increasing counter; each `load_image` call increments it.
        /// Async callbacks capture the value at dispatch time and discard their
        /// result if it no longer matches (i.e. a newer load was requested).
        pub load_gen: Cell<u64>,
    }

    impl Default for ViewerPane {
        fn default() -> Self {
            let picture = gtk4::Picture::new();
            picture.set_content_fit(gtk4::ContentFit::Contain);
            picture.set_can_shrink(true);
            picture.set_halign(gtk4::Align::Center);
            picture.set_valign(gtk4::Align::Center);

            let scrolled_window = gtk4::ScrolledWindow::new();
            scrolled_window.set_hexpand(true);
            scrolled_window.set_vexpand(true);
            scrolled_window.set_hscrollbar_policy(gtk4::PolicyType::Automatic);
            scrolled_window.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
            scrolled_window.set_child(Some(&picture));

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

            let progress_bar = gtk4::ProgressBar::new();
            progress_bar.add_css_class("osd");
            progress_bar.set_halign(gtk4::Align::Fill);
            progress_bar.set_valign(gtk4::Align::End);
            progress_bar.set_visible(false);

            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(&scrolled_window));
            overlay.add_overlay(&metadata_chip);
            overlay.add_overlay(&spinner);
            overlay.add_overlay(&progress_bar);

            let comparison = BeforeAfterViewer::new();

            let stack = gtk4::Stack::new();
            stack.set_hexpand(true);
            stack.set_vexpand(true);
            stack.add_named(&overlay, Some("view"));
            stack.add_named(&comparison, Some("compare"));

            Self {
                stack,
                overlay,
                scrolled_window,
                picture,
                metadata_chip,
                spinner,
                progress_bar,
                comparison,
                pending_output: std::cell::RefCell::new(None),
                commit_btn: std::cell::RefCell::new(None),
                discard_btn: std::cell::RefCell::new(None),
                zoom: Cell::new(1.0),
                metadata_visible: Cell::new(true),
                state: RefCell::new(None),
                pointer_pos: Cell::new((0.0, 0.0)),
                drag_origin: Cell::new(None),
                drag_adjustments: Cell::new((0.0, 0.0)),
                load_gen: Cell::new(0),
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
            self.stack.unparent();
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

    /// Store the Commit/Discard buttons so they can be shown/hidden during
    /// the upscale comparison flow. Called once by the window after layout.
    pub fn set_comparison_buttons(&self, commit: gtk4::Button, discard: gtk4::Button) {
        *self.imp().commit_btn.borrow_mut() = Some(commit);
        *self.imp().discard_btn.borrow_mut() = Some(discard);
    }

    fn build_ui(&self) {
        let imp = self.imp();
        imp.stack.set_parent(self);

        let motion = gtk4::EventControllerMotion::new();
        let w = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().pointer_pos.set((x, y));
            }
        });
        imp.scrolled_window.add_controller(motion);

        let drag = gtk4::GestureDrag::new();
        drag.set_button(0);

        let w = self.downgrade();
        drag.connect_drag_begin(move |_, start_x, start_y| {
            if let Some(viewer) = w.upgrade() {
                let imp = viewer.imp();
                imp.pointer_pos.set((start_x, start_y));
                imp.drag_origin.set(Some((start_x, start_y)));
                imp.drag_adjustments.set((
                    imp.scrolled_window.hadjustment().value(),
                    imp.scrolled_window.vadjustment().value(),
                ));
            }
        });

        let w = self.downgrade();
        drag.connect_drag_update(move |_, offset_x, offset_y| {
            if let Some(viewer) = w.upgrade() {
                viewer.pan_drag(offset_x, offset_y);
            }
        });

        let w = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().drag_origin.set(None);
            }
        });
        imp.scrolled_window.add_controller(drag);

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
            gtk4::EventControllerScrollFlags::VERTICAL | gtk4::EventControllerScrollFlags::KINETIC,
        );
        let w = self.downgrade();
        scroll.connect_scroll(move |ctrl, _dx, dy| {
            if ctrl
                .current_event_state()
                .contains(gdk4::ModifierType::CONTROL_MASK)
            {
                if let Some(viewer) = w.upgrade() {
                    if let Some((x, y)) = ctrl.current_event().and_then(|event| event.position()) {
                        viewer.imp().pointer_pos.set((x, y));
                    }
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
        imp.load_gen.set(imp.load_gen.get().wrapping_add(1));
        self.restore_view_mode();
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
        let load_gen = imp.load_gen.get().wrapping_add(1);
        imp.load_gen.set(load_gen);
        self.restore_view_mode();
        imp.spinner.start();
        imp.spinner.set_visible(true);
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();

        // Channel for decoded pixel data: (rgba_bytes, width, height)
        let (pixel_tx, pixel_rx) = async_channel::bounded::<Option<(Vec<u8>, u32, u32)>>(1);

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
        let (meta_tx, meta_rx) = async_channel::bounded::<crate::metadata::ImageMetadata>(1);
        std::thread::spawn(move || {
            let metadata = crate::metadata::ImageMetadata::load(&path_meta);
            let _ = meta_tx.send_blocking(metadata);
        });
        let widget_weak_meta = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(metadata) = meta_rx.recv().await {
                if let Some(viewer) = widget_weak_meta.upgrade() {
                    if viewer.imp().load_gen.get() != load_gen {
                        return;
                    }
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
            if viewer.imp().load_gen.get() != load_gen {
                return;
            }
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
        let old_zoom = imp.zoom.get();
        let new_zoom = (old_zoom * factor).clamp(0.05, 20.0);
        if (new_zoom - old_zoom).abs() < f64::EPSILON {
            return;
        }

        let hadj = imp.scrolled_window.hadjustment();
        let vadj = imp.scrolled_window.vadjustment();
        let (focus_x, focus_y) = imp.pointer_pos.get();
        let content_focus_x = hadj.value() + focus_x;
        let content_focus_y = vadj.value() + focus_y;
        imp.zoom.set(new_zoom);

        let Some(paintable) = imp.picture.paintable() else {
            return;
        };

        let base_width = paintable.intrinsic_width().max(1);
        let base_height = paintable.intrinsic_height().max(1);
        let scaled_width = (base_width as f64 * new_zoom).round().max(1.0) as i32;
        let scaled_height = (base_height as f64 * new_zoom).round().max(1.0) as i32;

        imp.picture.set_size_request(scaled_width, scaled_height);

        let scale_ratio = new_zoom / old_zoom;
        self.set_adjustment_value(
            &hadj,
            content_focus_x * scale_ratio - focus_x,
        );
        self.set_adjustment_value(
            &vadj,
            content_focus_y * scale_ratio - focus_y,
        );
    }

    fn reset_zoom(&self) {
        let imp = self.imp();
        imp.zoom.set(1.0);
        imp.picture.set_size_request(-1, -1);
        imp.scrolled_window.hadjustment().set_value(0.0);
        imp.scrolled_window.vadjustment().set_value(0.0);
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

    fn pan_drag(&self, offset_x: f64, offset_y: f64) {
        let imp = self.imp();
        let (start_h, start_v) = imp.drag_adjustments.get();

        self.set_adjustment_value(&imp.scrolled_window.hadjustment(), start_h - offset_x);
        self.set_adjustment_value(&imp.scrolled_window.vadjustment(), start_v - offset_y);
    }

    fn set_adjustment_value(&self, adjustment: &gtk4::Adjustment, value: f64) {
        let max_value = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value(value.clamp(adjustment.lower(), max_value));
    }

    // -----------------------------------------------------------------------
    // Upscaling (Phase B)
    // -----------------------------------------------------------------------

    /// Spawn an upscale job for `path`, using the binary cached in `AppState`.
    ///
    /// Disables `trigger_btn` for the duration; re-enables on completion or
    /// failure. On success, shows a comparison view backed by a temp output.
    pub fn start_upscale(&self, path: PathBuf, trigger_btn: gtk4::Button) {
        use crate::model::ImageEntry;
        use crate::upscale::runner::{UpscaleEvent, UpscaleRunner};

        let imp = self.imp();

        let binary = {
            let st = imp.state.borrow();
            let Some(ref rc) = *st else {
                trigger_btn.set_sensitive(true);
                return;
            };
            let b = rc.borrow().upscale_binary.clone();
            b
        };
        let Some(binary) = binary else {
            trigger_btn.set_sensitive(true);
            return;
        };

        let scale = {
            let st = imp.state.borrow();
            let Some(ref rc) = *st else { return };
            let (w, h) = rc
                .borrow()
                .library
                .selected_entry()
                .and_then(|e: ImageEntry| e.dimensions())
                .unwrap_or((0, 0));
            UpscaleRunner::smart_scale(w, h)
        };

        let final_output = {
            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let name = path.file_name().unwrap_or_default();
            parent.join("upscaled").join(name)
        };
        if let Some(dir) = final_output.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let output = pending_output_path(&final_output);

        imp.progress_bar.set_fraction(0.0);
        imp.progress_bar.set_visible(true);

        let rx = UpscaleRunner::run(&binary, &path, &output, scale);
        let widget_weak = self.downgrade();
        let btn_weak = trigger_btn.downgrade();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = rx.recv().await {
                let Some(viewer) = widget_weak.upgrade() else { break };
                let vimp = viewer.imp();
                match event {
                    UpscaleEvent::Progress(Some(f)) => {
                        vimp.progress_bar.set_fraction(f as f64);
                    }
                    UpscaleEvent::Progress(None) => {
                        vimp.progress_bar.pulse();
                    }
                    UpscaleEvent::Done(out_path) => {
                        vimp.progress_bar.set_visible(false);
                        if let Some(btn) = btn_weak.upgrade() {
                            btn.set_sensitive(true);
                        }
                        viewer.show_comparison(path.clone(), out_path);
                        break;
                    }
                    UpscaleEvent::Failed(msg) => {
                        vimp.progress_bar.set_visible(false);
                        eprintln!("Upscale failed: {msg}");
                        if let Some(btn) = btn_weak.upgrade() {
                            btn.set_sensitive(true);
                        }
                        break;
                    }
                }
            }
        });
    }

    /// Load both images into the comparison widget, show Commit/Discard, and
    /// switch the stack to the "compare" page.
    fn show_comparison(&self, before_path: PathBuf, after_path: PathBuf) {
        let imp = self.imp();
        *imp.pending_output.borrow_mut() = Some(after_path.clone());
        imp.comparison.load(before_path, after_path);
        self.set_compare_actions_visible(true);
        imp.stack.set_visible_child_name("compare");
    }

    /// Commit: load the upscaled output into the viewer and return to the
    /// normal view. Does NOT copy the file — the output path IS the final
    /// location (`<src_dir>/upscaled/<name>`).
    pub fn commit_upscale(&self) {
        let imp = self.imp();
        let pending_path = imp.pending_output.borrow_mut().take();
        self.restore_view_mode();
        if let Some(path) = pending_path {
            let final_path = committed_output_path(&path);
            if final_path != path {
                if std::fs::rename(&path, &final_path).is_ok() {
                    self.load_image(final_path);
                    return;
                }
            }
            self.load_image(path);
        }
    }

    /// Discard: delete the temp output file and return to the normal viewer.
    pub fn discard_upscale(&self) {
        let imp = self.imp();
        let out_path = imp.pending_output.borrow_mut().take();
        if let Some(path) = out_path {
            let _ = std::fs::remove_file(&path);
        }
        self.restore_view_mode();
    }

    fn restore_view_mode(&self) {
        let imp = self.imp();
        *imp.pending_output.borrow_mut() = None;
        imp.comparison.clear();
        self.set_compare_actions_visible(false);
        imp.stack.set_visible_child_name("view");
    }

    fn set_compare_actions_visible(&self, visible: bool) {
        let imp = self.imp();
        if let Some(ref btn) = *imp.commit_btn.borrow() {
            btn.set_visible(visible);
        }
        if let Some(ref btn) = *imp.discard_btn.borrow() {
            btn.set_visible(visible);
        }
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

fn pending_output_path(final_output: &std::path::Path) -> PathBuf {
    let stem = final_output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("upscaled");
    let ext = final_output.extension().and_then(|s| s.to_str()).unwrap_or("");
    let suffix = format!("{}.pending-{}", stem, std::process::id());
    let file_name = if ext.is_empty() {
        suffix
    } else {
        format!("{suffix}.{ext}")
    };
    final_output.with_file_name(file_name)
}

fn committed_output_path(pending_output: &std::path::Path) -> PathBuf {
    let name = pending_output
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let trimmed = match name.split_once(".pending-") {
        Some((prefix, rest)) => match rest.split_once('.') {
            Some((_, ext)) => format!("{prefix}.{ext}"),
            None => prefix.to_owned(),
        },
        None => name.to_owned(),
    };
    pending_output.with_file_name(trimmed)
}
