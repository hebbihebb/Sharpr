use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Once;

use crate::quality::{scorer, QualityScore};
use crate::ui::metadata_chip::MetadataChip;
use crate::ui::state::AppState;

// ---------------------------------------------------------------------------
// ViewerPane
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ZoomMode {
    Fit,
    OneToOne,
}

mod imp {
    use super::*;

    pub struct ViewerPane {
        pub content_box: gtk4::Box,
        pub overlay: gtk4::Overlay,
        pub scrolled_window: gtk4::ScrolledWindow,
        pub picture: gtk4::Picture,
        pub metadata_chip: MetadataChip,
        pub spinner: gtk4::Spinner,
        /// Shown briefly when a decode error prevents the image from loading.
        pub error_label: gtk4::Label,
        pub zoom_label: gtk4::Label,
        pub zoom_hide_source: RefCell<Option<glib::SourceId>>,
        /// Path of the image currently displayed — set by load_image(), cleared by clear().
        pub current_path: RefCell<Option<PathBuf>>,
        /// Zoom/Fit toggle button in the header — stored so mode changes can
        /// update the icon without an extra signal.
        pub zoom_btn: std::cell::RefCell<Option<gtk4::Button>>,
        pub zoom: Cell<f64>,
        pub zoom_mode: Cell<super::ZoomMode>,
        pub metadata_visible: Cell<bool>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub pointer_pos: Cell<(f64, f64)>,
        /// Look prototype: hides the corner chip after the pointer rests.
        pub chip_idle_source: RefCell<Option<glib::SourceId>>,
        pub chip_hovered: Cell<bool>,
        pub drag_origin: Cell<Option<(f64, f64)>>,
        pub drag_adjustments: Cell<(f64, f64)>,
        /// Monotonically increasing counter; each `load_image` call increments it.
        /// Async callbacks capture the value at dispatch time and discard their
        /// result if it no longer matches (i.e. a newer load was requested).
        pub load_gen: Cell<u64>,
        /// The original size while a screen-sized copy of a very large image
        /// is shown (`crate::display_scale`); zoom math uses this size.
        pub full_size: Cell<Option<(i32, i32)>>,
        /// A full-resolution load for zooming in has been started.
        pub full_res_requested: Cell<bool>,
        pub info_pending: Cell<bool>,
        pub membership_gen: Cell<u64>,
        /// Handle for submitting metadata load requests to the shared metadata worker.
        pub metadata_handle: RefCell<Option<crate::image_pipeline::worker::MetadataHandle>>,
        /// Last metadata-only quality score set for the displayed image, kept so
        /// `apply_sharpness` can blend without re-reading metadata.
        pub base_quality: RefCell<Option<crate::quality::QualityScore>>,
    }

    impl Default for ViewerPane {
        fn default() -> Self {
            let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

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

            let error_label = gtk4::Label::new(None);
            error_label.add_css_class("osd");
            error_label.set_halign(gtk4::Align::Center);
            error_label.set_valign(gtk4::Align::Center);
            error_label.set_wrap(true);
            error_label.set_visible(false);

            let metadata_chip = MetadataChip::new();
            metadata_chip.set_halign(gtk4::Align::End);
            metadata_chip.set_valign(gtk4::Align::End);
            metadata_chip.set_margin_end(16);
            metadata_chip.set_margin_bottom(4);

            let zoom_label = gtk4::Label::new(None);
            zoom_label.add_css_class("osd");
            zoom_label.add_css_class("title-2");
            zoom_label.add_css_class("zoom-osd");
            zoom_label.set_halign(gtk4::Align::Center);
            zoom_label.set_valign(gtk4::Align::Center);
            zoom_label.set_margin_start(24);
            zoom_label.set_margin_end(24);
            zoom_label.set_margin_top(16);
            zoom_label.set_margin_bottom(16);
            zoom_label.set_visible(false);

            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(&scrolled_window));
            overlay.add_overlay(&metadata_chip);
            overlay.add_overlay(&spinner);
            overlay.add_overlay(&error_label);
            overlay.add_overlay(&zoom_label);

            content_box.append(&overlay);

            Self {
                content_box,
                overlay,
                scrolled_window,
                picture,
                metadata_chip,
                spinner,
                error_label,
                zoom_label,
                zoom_hide_source: RefCell::new(None),
                current_path: RefCell::new(None),
                zoom_btn: std::cell::RefCell::new(None),
                zoom: Cell::new(1.0),
                zoom_mode: Cell::new(super::ZoomMode::Fit),
                metadata_visible: Cell::new(true),
                state: RefCell::new(None),
                pointer_pos: Cell::new((0.0, 0.0)),
                chip_idle_source: RefCell::new(None),
                chip_hovered: Cell::new(false),
                drag_origin: Cell::new(None),
                drag_adjustments: Cell::new((0.0, 0.0)),
                load_gen: Cell::new(0),
                full_size: Cell::new(None),
                full_res_requested: Cell::new(false),
                info_pending: Cell::new(false),
                membership_gen: Cell::new(0),
                metadata_handle: RefCell::new(None),
                base_quality: RefCell::new(None),
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
            self.content_box.unparent();
        }
    }

    impl WidgetImpl for ViewerPane {}
}

glib::wrapper! {
    pub struct ViewerPane(ObjectSubclass<imp::ViewerPane>)
        @extends gtk4::Widget,
                 @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ViewerPane {
    pub fn new(state: Rc<RefCell<AppState>>) -> Self {
        let widget: Self = glib::Object::new();
        *widget.imp().state.borrow_mut() = Some(state);
        widget.build_ui();
        widget
    }

    /// Look prototype: shows the corner chip, then fades it after 1.5 s
    /// without pointer movement, unless the pointer is on it or Info is open.
    pub fn reveal_chip(&self) {
        let imp = self.imp();
        imp.metadata_chip.remove_css_class("sharpr-idle");
        imp.metadata_chip.set_can_target(true);
        if let Some(source) = imp.chip_idle_source.borrow_mut().take() {
            source.remove();
        }
        let w = self.downgrade();
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                let Some(viewer) = w.upgrade() else { return };
                let imp = viewer.imp();
                imp.chip_idle_source.borrow_mut().take();
                if imp.chip_hovered.get() || imp.metadata_chip.imp().popover.is_visible() {
                    return;
                }
                imp.metadata_chip.add_css_class("sharpr-idle");
                imp.metadata_chip.set_can_target(false);
            });
        *imp.chip_idle_source.borrow_mut() = Some(source);
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.imp().current_path.borrow().clone()
    }

    fn build_ui(&self) {
        let imp = self.imp();
        imp.content_box.set_parent(self);
        install_viewer_osd_css();

        let motion = gtk4::EventControllerMotion::new();
        let w = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().pointer_pos.set((x, y));
            }
        });
        imp.scrolled_window.add_controller(motion);

        // Look prototype: images sit on a neutral canvas, and the corner chip
        // shows while the pointer moves and fades when it rests (I12, I14).
        imp.overlay.add_css_class("sharpr-canvas");
        imp.metadata_chip.add_css_class("sharpr-chip");
        let hover = gtk4::EventControllerMotion::new();
        let w = self.downgrade();
        hover.connect_enter(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().chip_hovered.set(true);
            }
        });
        let w = self.downgrade();
        hover.connect_leave(move |_| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().chip_hovered.set(false);
                viewer.reveal_chip();
            }
        });
        imp.metadata_chip.add_controller(hover);
        let wake = gtk4::EventControllerMotion::new();
        let w = self.downgrade();
        wake.connect_motion(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                viewer.reveal_chip();
            }
        });
        imp.overlay.add_controller(wake);
        self.reveal_chip();

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

        // Double-click → toggle presentation mode
        let dblclick = gtk4::GestureClick::new();
        dblclick.set_button(1);
        let w = self.downgrade();
        dblclick.connect_released(move |_, n_press, _, _| {
            if n_press == 2 {
                if let Some(viewer) = w.upgrade() {
                    let _ = viewer.activate_action("win.toggle-presentation", None);
                }
            }
        });
        imp.scrolled_window.add_controller(dblclick);

        // -----------------------------------------------------------------------
        // Keyboard shortcuts
        // -----------------------------------------------------------------------
        let shortcuts = gtk4::ShortcutController::new();
        shortcuts.set_scope(gtk4::ShortcutScope::Managed);

        // Ctrl+0 — reset zoom
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Control>0").unwrap()),
            Some(gtk4::CallbackAction::new(move |widget, _| {
                let _ = widget.activate_action("win.zoom-mode", Some(&"fit".to_variant()));
                glib::Propagation::Stop
            })),
        ));

        // Alt+Return — toggle metadata overlay
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            Some(gtk4::ShortcutTrigger::parse_string("<Alt>Return").unwrap()),
            Some(gtk4::NamedAction::new("win.show-metadata")),
        ));

        self.add_controller(shortcuts);

        // -----------------------------------------------------------------------
        // Scroll → zoom
        // -----------------------------------------------------------------------
        // Capture phase so we intercept scroll before the ScrolledWindow
        // consumes it (which it does whenever there is scrollable overflow).
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let w = self.downgrade();
        scroll.connect_scroll(move |ctrl, _dx, dy| {
            if let Some(viewer) = w.upgrade() {
                if let Some((x, y)) = ctrl.current_event().and_then(|event| event.position()) {
                    viewer.imp().pointer_pos.set((x, y));
                }
                let factor = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
                viewer.apply_zoom(factor);
            }
            glib::Propagation::Stop
        });
        self.add_controller(scroll);

        let w = self.downgrade();
        self.connect_map(move |_| {
            if let Some(viewer) = w.upgrade() {
                viewer.ensure_fit_on_layout();
            }
        });
    }

    // -----------------------------------------------------------------------
    // Image loading (async via background thread + idle callback)
    // -----------------------------------------------------------------------

    /// Call once from window setup. Metadata results are drained on the GTK
    /// main thread; stale results are discarded by generation comparison.
    pub fn set_metadata_worker(
        &self,
        handle: crate::image_pipeline::worker::MetadataHandle,
        result_rx: async_channel::Receiver<crate::image_pipeline::worker::MetadataResult>,
    ) {
        *self.imp().metadata_handle.borrow_mut() = Some(handle);

        let widget_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = result_rx.recv().await {
                let Some(viewer) = widget_weak.upgrade() else {
                    break;
                };
                let imp = viewer.imp();
                if result.gen != imp.load_gen.get() {
                    continue;
                }
                imp.metadata_chip.update_metadata(&result.metadata);
                viewer.update_quality_indicator(&result.metadata);
                if imp.info_pending.replace(false) {
                    imp.metadata_chip.show_info();
                }
                viewer.refresh_membership();
            }
        });
    }

    /// Opens the image Info panel, now or as soon as the current image's
    /// details have loaded (decision D18).
    pub fn show_info(&self) {
        let imp = self.imp();
        if !imp.metadata_chip.show_info() {
            imp.info_pending.set(imp.current_path.borrow().is_some());
        }
    }

    /// Clear the viewer (called when the folder changes).
    pub fn clear(&self) {
        let imp = self.imp();
        imp.load_gen.set(imp.load_gen.get().wrapping_add(1));
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();
        imp.metadata_chip.update_collections(&[]);
        imp.metadata_chip.update_duplicate_count(0);
        self.set_quality_score(None);
        imp.spinner.stop();
        imp.spinner.set_visible(false);
        imp.error_label.set_visible(false);
        self.reset_zoom();
        *imp.current_path.borrow_mut() = None;
    }

    /// Load and display a full-resolution image from `path`.
    ///
    /// The image is loaded asynchronously using `glycin::Loader` on the GTK
    /// main thread. This leverages sandboxed decoding and supports high
    /// bit-depth formats like JPEG XL.
    pub fn load_image(&self, path: PathBuf) {
        if self.imp().current_path.borrow().as_ref() == Some(&path) {
            return;
        }
        crate::bench_event!(
            "viewer.load.request",
            serde_json::json!({
                "path": path.display().to_string(),
            }),
        );
        let imp = self.imp();
        let load_gen = imp.load_gen.get().wrapping_add(1);
        imp.load_gen.set(load_gen);
        imp.full_size.set(None);
        imp.full_res_requested.set(false);
        *imp.current_path.borrow_mut() = Some(path.clone());
        imp.spinner.start();
        imp.spinner.set_visible(true);
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();
        imp.metadata_chip.update_location(&path);
        imp.metadata_chip.update_collections(&[]);
        imp.metadata_chip.update_duplicate_count(0);
        imp.error_label.set_visible(false);
        self.set_quality_score(None);
        self.refresh_membership();

        let picture = imp.picture.clone();
        let spinner = imp.spinner.clone();
        let error_label = imp.error_label.clone();
        let path_clone = path.clone();
        let viewer = self.clone();
        let screen = self.screen_pixels();

        glib::MainContext::default().spawn_local(async move {
            let started = std::time::Instant::now();
            let file = gio::File::for_path(&path_clone);
            let result = async {
                let loader = glycin::Loader::new(file);
                let image = loader.load().await.map_err(|e| e.to_string())?;
                let frame = image.next_frame().await.map_err(|e| e.to_string())?;
                let (width, height) = (frame.width(), frame.height());
                // sRGB only: other colour states would need glycin's private
                // conversion, and are rare enough to show as they are.
                let copy_plan = crate::display_scale::bytes_per_pixel(frame.memory_format())
                    .zip(crate::display_scale::display_size(width, height, screen))
                    .filter(|_| matches!(frame.color_state(), glycin::ColorState::Srgb));
                let Some((bpp, target)) = copy_plan else {
                    return Ok::<_, String>((frame.texture(), None));
                };
                // Very large: show a screen-sized copy made on a worker, so GTK
                // never builds or uploads the full image unless the user zooms in.
                let format = glycin::gdk_memory_format(frame.memory_format());
                let color_state = gdk4::ColorState::srgb();
                let bytes = frame.buf_bytes();
                let stride = frame.stride() as usize;
                drop(frame);
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let pixels =
                        crate::display_scale::downscale(&bytes, width, height, stride, bpp, target);
                    let _ = tx.send_blocking(pixels);
                });
                let pixels = rx.recv().await.map_err(|e| e.to_string())?;
                let copy = gdk4::MemoryTextureBuilder::new()
                    .set_bytes(Some(&glib::Bytes::from_owned(pixels)))
                    .set_width(target.0 as i32)
                    .set_height(target.1 as i32)
                    .set_stride(target.0 as usize * bpp)
                    .set_format(format)
                    .set_color_state(&color_state)
                    .build();
                Ok((copy, Some((width as i32, height as i32))))
            }
            .await;

            if viewer.imp().load_gen.get() != load_gen {
                return; // User navigated away
            }

            spinner.stop();
            spinner.set_visible(false);

            match result {
                Ok((texture, full_size)) => {
                    let (width, height) = full_size.unwrap_or((texture.width(), texture.height()));
                    crate::bench_event!(
                        "viewer.load.ready",
                        serde_json::json!({
                            "width": width,
                            "height": height,
                            "shown_width": texture.width(),
                            "shown_height": texture.height(),
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    viewer.imp().full_size.set(full_size);
                    picture.set_paintable(Some(texture.upcast_ref::<gdk4::Paintable>()));
                    viewer.reset_zoom();
                    viewer.ensure_fit_on_layout();
                }
                Err(e) => {
                    error_label.set_text(&format!("Could not load image: {}", e));
                    error_label.set_visible(true);
                }
            }
        });

        // Metadata loads via the shared worker; result drains in set_metadata_worker loop.
        if let Some(ref handle) = *imp.metadata_handle.borrow() {
            handle.request(path.clone(), load_gen);
        }
    }

    // -----------------------------------------------------------------------
    // Zoom
    // -----------------------------------------------------------------------------------

    fn apply_zoom(&self, factor: f64) {
        let imp = self.imp();
        let old_zoom = imp.zoom.get();
        let Some(paintable) = imp.picture.paintable() else {
            return;
        };
        let fit_scale = self.fit_scale_for_paintable(&paintable);
        let max_zoom = (1.0 / fit_scale.max(f64::EPSILON)).max(1.0) * 20.0;
        let new_zoom = (old_zoom * factor).clamp(1.0, max_zoom);
        if (new_zoom - old_zoom).abs() < f64::EPSILON {
            return;
        }

        let hadj = imp.scrolled_window.hadjustment();
        let vadj = imp.scrolled_window.vadjustment();
        let (focus_x, focus_y) = imp.pointer_pos.get();
        let content_focus_x = hadj.value() + focus_x;
        let content_focus_y = vadj.value() + focus_y;
        imp.zoom.set(new_zoom);

        let (base_width, base_height) = self.image_size(&paintable);
        let scaled_width = (base_width * fit_scale * new_zoom).round().max(1.0) as i32;
        let scaled_height = (base_height * fit_scale * new_zoom).round().max(1.0) as i32;

        imp.picture.set_size_request(scaled_width, scaled_height);
        self.ensure_resolution_for(&paintable, scaled_width);

        let scale_ratio = new_zoom / old_zoom;
        self.set_adjustment_value(&hadj, content_focus_x * scale_ratio - focus_x);
        self.set_adjustment_value(&vadj, content_focus_y * scale_ratio - focus_y);
        let pct = (new_zoom * 100.0).round() as u32;
        self.show_zoom_osd(&format!("{pct}%"));
    }

    pub fn ensure_fit_on_layout(&self) {
        let viewer = self.clone();
        glib::idle_add_local_once(move || {
            let imp = viewer.imp();
            if imp.zoom_mode.get() == ZoomMode::Fit {
                viewer.update_picture_zoom();
            }
        });
    }

    pub fn reset_zoom(&self) {
        let imp = self.imp();
        imp.zoom.set(1.0);
        imp.zoom_mode.set(ZoomMode::Fit);
        self.update_picture_zoom();
        imp.scrolled_window.hadjustment().set_value(0.0);
        imp.scrolled_window.vadjustment().set_value(0.0);
        self.sync_zoom_button();
    }

    pub fn set_zoom_mode(&self, mode: ZoomMode) {
        let imp = self.imp();
        imp.zoom_mode.set(mode);
        match mode {
            ZoomMode::Fit => {
                imp.zoom.set(1.0);
                self.update_picture_zoom();
                imp.scrolled_window.hadjustment().set_value(0.0);
                imp.scrolled_window.vadjustment().set_value(0.0);
            }
            ZoomMode::OneToOne => {
                let Some(paintable) = imp.picture.paintable() else {
                    return;
                };
                let fit_scale = self.fit_scale_for_paintable(&paintable);
                imp.zoom.set((1.0 / fit_scale.max(f64::EPSILON)).max(1.0));
                self.update_picture_zoom();
                // Centre the scroll on the image after layout.
                let hadj = imp.scrolled_window.hadjustment();
                let vadj = imp.scrolled_window.vadjustment();
                glib::idle_add_local_once({
                    let hadj = hadj.clone();
                    let vadj = vadj.clone();
                    move || {
                        hadj.set_value((hadj.upper() - hadj.page_size()) / 2.0);
                        vadj.set_value((vadj.upper() - vadj.page_size()) / 2.0);
                    }
                });
            }
        }
        if mode == ZoomMode::OneToOne {
            let pct = (imp.zoom.get() * 100.0).round() as u32;
            self.show_zoom_osd(&format!("{pct}%"));
        }
        self.sync_zoom_button();
    }

    /// Store the zoom/fit toggle button so mode changes can update its icon.
    pub fn set_zoom_button(&self, btn: gtk4::Button) {
        *self.imp().zoom_btn.borrow_mut() = Some(btn);
    }

    /// Toggle between Fit and 1:1 pixel mode. Updates the stored button icon.
    pub fn toggle_zoom_mode(&self) {
        let imp = self.imp();
        let new_mode = match imp.zoom_mode.get() {
            ZoomMode::Fit => ZoomMode::OneToOne,
            ZoomMode::OneToOne => ZoomMode::Fit,
        };
        imp.zoom_mode.set(new_mode);
        match new_mode {
            ZoomMode::Fit => {
                imp.zoom.set(1.0);
                self.update_picture_zoom();
                imp.scrolled_window.hadjustment().set_value(0.0);
                imp.scrolled_window.vadjustment().set_value(0.0);
            }
            ZoomMode::OneToOne => {
                let Some(paintable) = imp.picture.paintable() else {
                    return;
                };
                let fit_scale = self.fit_scale_for_paintable(&paintable);
                imp.zoom.set((1.0 / fit_scale.max(f64::EPSILON)).max(1.0));
                self.update_picture_zoom();
                // Centre the scroll on the image after layout.
                let hadj = imp.scrolled_window.hadjustment();
                let vadj = imp.scrolled_window.vadjustment();
                glib::idle_add_local_once({
                    let hadj = hadj.clone();
                    let vadj = vadj.clone();
                    move || {
                        hadj.set_value((hadj.upper() - hadj.page_size()) / 2.0);
                        vadj.set_value((vadj.upper() - vadj.page_size()) / 2.0);
                    }
                });
            }
        }
        if new_mode == ZoomMode::OneToOne {
            let pct = (imp.zoom.get() * 100.0).round() as u32;
            self.show_zoom_osd(&format!("{pct}%"));
        }
        self.sync_zoom_button();
    }

    /// The image's size in pixels: the original's, even while a screen-sized
    /// copy is shown, so zoom levels and 1:1 keep their meaning.
    fn image_size(&self, paintable: &gdk4::Paintable) -> (f64, f64) {
        match self.imp().full_size.get() {
            Some((width, height)) => (width.max(1) as f64, height.max(1) as f64),
            None => (
                paintable.intrinsic_width().max(1) as f64,
                paintable.intrinsic_height().max(1) as f64,
            ),
        }
    }

    /// The device pixels of the monitor the viewer is on (a 4K screen until
    /// the window is mapped).
    fn screen_pixels(&self) -> (u32, u32) {
        let monitor = self
            .native()
            .and_then(|native| native.surface())
            .and_then(|surface| self.display().monitor_at_surface(&surface));
        match monitor {
            Some(monitor) => {
                let geometry = monitor.geometry();
                let scale = monitor.scale_factor().max(1);
                (
                    (geometry.width() * scale).max(1) as u32,
                    (geometry.height() * scale).max(1) as u32,
                )
            }
            None => (3840, 2160),
        }
    }

    /// Loads full resolution once the picture, at `shown_width` logical pixels,
    /// needs more pixels than the screen-sized copy has. The slow upload then
    /// happens only because the user zoomed in.
    fn ensure_resolution_for(&self, paintable: &gdk4::Paintable, shown_width: i32) {
        let imp = self.imp();
        if imp.full_size.get().is_none() || imp.full_res_requested.get() {
            return;
        }
        let needed = shown_width as f64 * self.scale_factor().max(1) as f64;
        if needed <= paintable.intrinsic_width() as f64 * 1.02 {
            return;
        }
        let Some(path) = imp.current_path.borrow().clone() else {
            return;
        };
        imp.full_res_requested.set(true);
        let load_gen = imp.load_gen.get();
        let viewer = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let started = std::time::Instant::now();
            let result = async {
                let loader = glycin::Loader::new(gio::File::for_path(&path));
                let image = loader.load().await.map_err(|e| e.to_string())?;
                let frame = image.next_frame().await.map_err(|e| e.to_string())?;
                Ok::<_, String>(frame.texture())
            }
            .await;
            let imp = viewer.imp();
            if imp.load_gen.get() != load_gen {
                return;
            }
            match result {
                Ok(texture) => {
                    crate::bench_event!(
                        "viewer.full_resolution",
                        serde_json::json!({
                            "width": texture.width(),
                            "height": texture.height(),
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    // Same size as `full_size`, so the zoom and scroll stay put.
                    imp.full_size.set(None);
                    imp.picture
                        .set_paintable(Some(texture.upcast_ref::<gdk4::Paintable>()));
                }
                Err(error) => crate::bench_warn!(
                    "viewer.full_resolution.fail",
                    serde_json::json!({"error": error}),
                ),
            }
        });
    }

    fn fit_scale_for_paintable(&self, paintable: &gdk4::Paintable) -> f64 {
        let (base_width, base_height) = self.image_size(paintable);
        let imp = self.imp();
        let hadj = imp.scrolled_window.hadjustment();
        let vadj = imp.scrolled_window.vadjustment();
        let viewport_width = if hadj.page_size() > 0.0 {
            hadj.page_size()
        } else {
            imp.scrolled_window.width() as f64
        };
        let viewport_height = if vadj.page_size() > 0.0 {
            vadj.page_size()
        } else {
            imp.scrolled_window.height() as f64
        };

        if viewport_width <= 0.0 || viewport_height <= 0.0 {
            1.0
        } else {
            (viewport_width / base_width).min(viewport_height / base_height)
        }
    }

    fn update_picture_zoom(&self) {
        let imp = self.imp();
        let Some(paintable) = imp.picture.paintable() else {
            return;
        };
        let fit_scale = self.fit_scale_for_paintable(&paintable);
        let render_scale = (fit_scale * imp.zoom.get()).max(0.01);
        let (base_width, base_height) = self.image_size(&paintable);
        let width = (base_width * render_scale).round().max(1.0) as i32;
        let height = (base_height * render_scale).round().max(1.0) as i32;
        imp.picture.set_size_request(width, height);
        self.ensure_resolution_for(&paintable, width);
    }

    fn show_zoom_osd(&self, text: &str) {
        let imp = self.imp();
        imp.zoom_label.set_text(text);
        imp.zoom_label.set_visible(true);
        if let Some(source) = imp.zoom_hide_source.borrow_mut().take() {
            source.remove();
        }
        let w = self.downgrade();
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_millis(2000), move || {
                if let Some(viewer) = w.upgrade() {
                    let imp = viewer.imp();
                    imp.zoom_label.set_visible(false);
                    *imp.zoom_hide_source.borrow_mut() = None;
                }
            });
        *imp.zoom_hide_source.borrow_mut() = Some(source);
    }

    fn sync_zoom_button(&self) {
        let imp = self.imp();
        if let Some(ref btn) = *imp.zoom_btn.borrow() {
            match self.zoom_mode() {
                ZoomMode::Fit => {
                    btn.set_icon_name("zoom-fit-best-symbolic");
                    btn.set_tooltip_text(Some("1:1 Pixels (switch to actual size)"));
                }
                ZoomMode::OneToOne => {
                    btn.set_icon_name("zoom-original-symbolic");
                    btn.set_tooltip_text(Some("Fit to Window (switch to fit)"));
                }
            }
        }
    }

    pub fn metadata_visible(&self) -> bool {
        self.imp().metadata_visible.get()
    }

    pub fn set_metadata_visible(&self, visible: bool) {
        let imp = self.imp();
        imp.metadata_visible.set(visible);
        imp.metadata_chip.set_enabled(visible);
        if visible {
            self.refresh_membership();
        }
    }

    fn update_quality_indicator(&self, metadata: &crate::metadata::ImageMetadata) {
        self.set_quality_score(Some(&scorer::score_metadata(metadata)));
    }

    fn set_quality_score(&self, quality: Option<&QualityScore>) {
        let imp = self.imp();
        *imp.base_quality.borrow_mut() = quality.cloned();
        let Some(quality) = quality else {
            imp.metadata_chip.update_quality(None);
            return;
        };
        imp.metadata_chip.update_quality(Some(quality));
    }

    /// Refresh the quality display for the currently shown image after a
    /// sharpness result arrives. Resolution remains the displayed tier.
    pub fn apply_sharpness(&self, path: &std::path::Path, _sharpness_norm: f64) {
        let imp = self.imp();
        if imp.current_path.borrow().as_deref() != Some(path) {
            return;
        }
        let base = imp.base_quality.borrow().clone();
        let Some(base) = base else { return };
        imp.metadata_chip.update_quality(Some(&base));
    }

    pub fn zoom_mode(&self) -> ZoomMode {
        self.imp().zoom_mode.get()
    }

    // -----------------------------------------------------------------------
    // Metadata overlay
    // -----------------------------------------------------------------------

    pub fn toggle_metadata(&self) {
        self.set_metadata_visible(!self.metadata_visible());
    }

    fn refresh_membership(&self) {
        let imp = self.imp();
        let path = match imp.current_path.borrow().clone() {
            Some(path) => path,
            None => {
                imp.metadata_chip.update_collections(&[]);
                imp.metadata_chip.update_duplicate_count(0);
                return;
            }
        };

        let (index, library_id) = imp
            .state
            .borrow()
            .as_ref()
            .map(|state| {
                let st = state.borrow();
                (
                    st.library_index.clone(),
                    st.settings.active_library_id.clone(),
                )
            })
            .unwrap_or_default();
        let generation = imp.membership_gen.get().wrapping_add(1);
        imp.membership_gen.set(generation);
        imp.metadata_chip.update_collections(&[]);
        imp.metadata_chip.update_duplicate_count(0);
        let worker_path = path.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                // The active library's buckets, as the sidebar shows them.
                let collections = index
                    .as_ref()
                    .map(|index| index.list_collections_for_library(library_id.as_deref()))
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                let names = match index.as_ref() {
                    Some(index) => {
                        crate::buckets::buckets_for_path(index, &collections, &worker_path)?
                            .iter()
                            .map(|c| c.name.clone())
                            .collect::<Vec<_>>()
                    }
                    None => Vec::new(),
                };
                // Identical copies elsewhere, by content fingerprint (Phase 7).
                let duplicates = index
                    .as_ref()
                    .map(|index| crate::copies::identical_elsewhere(index, &worker_path))
                    .unwrap_or(0);
                Ok((names, duplicates))
            })();
            let _ = tx.send_blocking(result);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else { return };
            let Some(viewer) = weak.upgrade() else { return };
            let imp = viewer.imp();
            if imp.membership_gen.get() != generation
                || imp.current_path.borrow().as_ref() != Some(&path)
            {
                return;
            }
            match result {
                Ok((names, duplicates)) => {
                    imp.metadata_chip.update_collections(&names);
                    imp.metadata_chip.update_duplicate_count(duplicates);
                }
                Err(error) => {
                    crate::bench_warn!("viewer.membership.fail", serde_json::json!({"error":error}))
                }
            }
        });
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
}

fn install_viewer_osd_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_resource("/io/github/hebbihebb/Sharpr/viewer-osd.css");
        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}
