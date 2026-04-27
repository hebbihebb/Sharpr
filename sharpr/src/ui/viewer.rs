use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Once};
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::quality::{scorer, QualityScore};
use crate::tags::TagDatabase;
use crate::ui::metadata_chip::MetadataChip;
use crate::ui::window::AppState;
use crate::upscale::BeforeAfterViewer;

// ---------------------------------------------------------------------------
// ViewerPane
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ZoomMode {
    Fit,
    OneToOne,
}

#[derive(Default)]
struct TagPopoverState {
    db: Option<Arc<TagDatabase>>,
    path: Option<PathBuf>,
}

mod imp {
    use super::*;

    pub struct ViewerPane {
        pub content_box: gtk4::Box,
        /// Root stack: "view" = normal viewer, "compare" = before/after widget.
        pub stack: gtk4::Stack,
        pub overlay: gtk4::Overlay,
        pub scrolled_window: gtk4::ScrolledWindow,
        pub picture: gtk4::Picture,
        pub metadata_chip: MetadataChip,
        pub tag_osd: gtk4::Box,
        pub tag_label: gtk4::Label,
        pub tag_button: gtk4::Button,
        pub tag_add_button: gtk4::Button,
        pub spinner: gtk4::Spinner,
        /// Shown briefly when a decode error prevents the image from loading.
        pub error_label: gtk4::Label,
        /// OSD progress bar — shown during an upscale job, hidden otherwise.
        pub progress_bar: gtk4::ProgressBar,
        pub zoom_label: gtk4::Label,
        pub zoom_hide_source: RefCell<Option<glib::SourceId>>,
        pub tag_anchor: gtk4::Box,
        pub tag_popover: gtk4::Popover,
        pub tag_entry: gtk4::Entry,
        pub tag_flowbox: gtk4::FlowBox,
        pub smart_tag_btn: gtk4::Button,
        pub smart_tag_spinner: gtk4::Spinner,
        pub suggestions_box: gtk4::Box,
        pub suggestions_flow: gtk4::FlowBox,
        pub suggestions_add_all: gtk4::Button,
        pub(super) tag_state: Rc<RefCell<TagPopoverState>>,
        pub comparison: BeforeAfterViewer,
        /// Temp output path while the compare view is active.
        pub pending_output: std::cell::RefCell<Option<std::path::PathBuf>>,
        /// Commit/Discard buttons owned by the window header; stored here so
        /// async upscale callbacks can show/hide them without capturing clones.
        pub commit_btn: std::cell::RefCell<Option<gtk4::Button>>,
        pub discard_btn: std::cell::RefCell<Option<gtk4::Button>>,
        /// Path of the image currently displayed — set by load_image(), cleared by clear().
        pub current_path: RefCell<Option<PathBuf>>,
        /// Canonical decoded RGBA pixels for the currently displayed image.
        pub current_rgba: RefCell<Option<(Vec<u8>, u32, u32)>>,
        /// True when apply_transform() has been called and the result is unsaved.
        pub pending_edit: Cell<bool>,
        /// Edit Save/Discard buttons owned by the window header.
        pub edit_commit_btn: RefCell<Option<gtk4::Button>>,
        pub edit_discard_btn: RefCell<Option<gtk4::Button>>,
        /// Zoom/Fit toggle button in the header — stored so mode changes can
        /// update the icon without an extra signal.
        pub zoom_btn: std::cell::RefCell<Option<gtk4::Button>>,
        pub zoom: Cell<f64>,
        pub zoom_mode: Cell<super::ZoomMode>,
        pub metadata_visible: Cell<bool>,
        pub state: RefCell<Option<Rc<RefCell<AppState>>>>,
        pub pointer_pos: Cell<(f64, f64)>,
        pub drag_origin: Cell<Option<(f64, f64)>>,
        pub drag_adjustments: Cell<(f64, f64)>,
        /// Monotonically increasing counter; each `load_image` call increments it.
        /// Async callbacks capture the value at dispatch time and discard their
        /// result if it no longer matches (i.e. a newer load was requested).
        pub load_gen: Cell<u64>,
        /// Handle for submitting decode requests to the shared preview worker pool.
        pub preview_handle: RefCell<Option<crate::image_pipeline::worker::PreviewHandle>>,
        /// Handle for submitting metadata load requests to the shared metadata worker.
        pub metadata_handle: RefCell<Option<crate::image_pipeline::worker::MetadataHandle>>,
        /// Called after a successful edit save so the filmstrip can refresh thumbnails.
        pub post_save_cb: RefCell<Option<Box<dyn Fn()>>>,
        /// Commit action for the active convert operation (downscale or upscale).
        /// Called with the temp output path when the user clicks Commit.
        pub pending_commit_fn: RefCell<Option<Box<dyn FnOnce(PathBuf)>>>,
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

            let tag_osd = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            tag_osd.add_css_class("osd");
            tag_osd.add_css_class("tag-osd");
            tag_osd.set_halign(gtk4::Align::Start);
            tag_osd.set_valign(gtk4::Align::End);
            tag_osd.set_margin_start(16);
            tag_osd.set_margin_bottom(16);
            tag_osd.set_visible(false);

            let tag_button = gtk4::Button::new();
            tag_button.add_css_class("flat");
            tag_button.add_css_class("tag-osd-pill");
            tag_button.set_focus_on_click(false);
            tag_button.set_tooltip_text(Some("Edit tags"));

            let tag_label = gtk4::Label::new(Some("Tags"));
            tag_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            tag_label.set_max_width_chars(18);
            tag_label.set_xalign(0.0);
            tag_button.set_child(Some(&tag_label));

            let tag_add_button = gtk4::Button::from_icon_name("list-add-symbolic");
            tag_add_button.add_css_class("flat");
            tag_add_button.add_css_class("tag-osd-add");
            tag_add_button.set_focus_on_click(false);
            tag_add_button.set_tooltip_text(Some("Add or edit tags"));

            tag_osd.append(&tag_button);
            tag_osd.append(&tag_add_button);

            let progress_bar = gtk4::ProgressBar::new();
            progress_bar.add_css_class("osd");
            progress_bar.set_halign(gtk4::Align::Fill);
            progress_bar.set_valign(gtk4::Align::End);
            progress_bar.set_visible(false);

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

            let tag_anchor = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            tag_anchor.set_halign(gtk4::Align::End);
            tag_anchor.set_valign(gtk4::Align::Start);
            tag_anchor.set_margin_top(12);
            tag_anchor.set_margin_end(12);
            tag_anchor.set_size_request(1, 1);

            let tag_popover = gtk4::Popover::new();
            tag_popover.set_has_arrow(true);
            tag_popover.set_autohide(true);
            tag_popover.set_position(gtk4::PositionType::Bottom);

            let tag_entry = gtk4::Entry::new();
            let tag_flowbox = gtk4::FlowBox::new();
            let smart_tag_btn = gtk4::Button::from_icon_name("starred-symbolic");
            smart_tag_btn.add_css_class("flat");
            smart_tag_btn.set_tooltip_text(Some("Suggest tags with AI"));
            smart_tag_btn.set_visible(false);

            let smart_tag_spinner = gtk4::Spinner::new();
            smart_tag_spinner.set_visible(false);
            smart_tag_spinner.set_size_request(16, 16);

            let suggestions_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            suggestions_box.set_visible(false);

            let suggestions_flow = gtk4::FlowBox::new();
            suggestions_flow.set_selection_mode(gtk4::SelectionMode::None);
            suggestions_flow.set_row_spacing(4);
            suggestions_flow.set_column_spacing(4);
            suggestions_flow.set_homogeneous(false);
            suggestions_flow.set_valign(gtk4::Align::Start);

            let suggestions_add_all = gtk4::Button::with_label("Add All");
            suggestions_add_all.add_css_class("suggested-action");
            suggestions_add_all.set_halign(gtk4::Align::End);
            suggestions_add_all.set_visible(false);

            let tag_state = Rc::new(RefCell::new(TagPopoverState::default()));

            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(&scrolled_window));
            overlay.add_overlay(&tag_anchor);
            overlay.add_overlay(&tag_osd);
            overlay.add_overlay(&metadata_chip);
            overlay.add_overlay(&spinner);
            overlay.add_overlay(&error_label);
            overlay.add_overlay(&progress_bar);
            overlay.add_overlay(&zoom_label);

            let comparison = BeforeAfterViewer::new();

            let stack = gtk4::Stack::new();
            stack.set_hexpand(true);
            stack.set_vexpand(true);
            stack.add_named(&overlay, Some("view"));
            stack.add_named(&comparison, Some("compare"));

            content_box.append(&stack);

            Self {
                content_box,
                stack,
                overlay,
                scrolled_window,
                picture,
                metadata_chip,
                tag_osd,
                tag_label,
                tag_button,
                tag_add_button,
                spinner,
                error_label,
                progress_bar,
                zoom_label,
                zoom_hide_source: RefCell::new(None),
                tag_anchor,
                tag_popover,
                tag_entry,
                tag_flowbox,
                smart_tag_btn,
                smart_tag_spinner,
                suggestions_box,
                suggestions_flow,
                suggestions_add_all,
                tag_state,
                comparison,
                pending_output: std::cell::RefCell::new(None),
                commit_btn: std::cell::RefCell::new(None),
                discard_btn: std::cell::RefCell::new(None),
                current_path: RefCell::new(None),
                current_rgba: RefCell::new(None),
                pending_edit: Cell::new(false),
                edit_commit_btn: RefCell::new(None),
                edit_discard_btn: RefCell::new(None),
                zoom_btn: std::cell::RefCell::new(None),
                zoom: Cell::new(1.0),
                zoom_mode: Cell::new(super::ZoomMode::Fit),
                metadata_visible: Cell::new(true),
                state: RefCell::new(None),
                pointer_pos: Cell::new((0.0, 0.0)),
                drag_origin: Cell::new(None),
                drag_adjustments: Cell::new((0.0, 0.0)),
                load_gen: Cell::new(0),
                preview_handle: RefCell::new(None),
                metadata_handle: RefCell::new(None),
                post_save_cb: RefCell::new(None),
                pending_commit_fn: RefCell::new(None),
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

    /// Called once by the window after layout to store the edit Save/Discard buttons.
    pub fn set_edit_buttons(&self, commit: gtk4::Button, discard: gtk4::Button) {
        *self.imp().edit_commit_btn.borrow_mut() = Some(commit);
        *self.imp().edit_discard_btn.borrow_mut() = Some(discard);
    }

    fn set_comparison_buttons_visible(&self, visible: bool) {
        let imp = self.imp();
        if let Some(ref btn) = *imp.commit_btn.borrow() {
            btn.set_visible(visible);
        }
        if let Some(ref btn) = *imp.discard_btn.borrow() {
            btn.set_visible(visible);
        }
    }

    fn set_edit_buttons_visible_on(imp: &imp::ViewerPane, visible: bool) {
        if let Some(ref btn) = *imp.edit_commit_btn.borrow() {
            btn.set_visible(visible);
        }
        if let Some(ref btn) = *imp.edit_discard_btn.borrow() {
            btn.set_visible(visible);
        }
    }

    fn build_ui(&self) {
        let imp = self.imp();
        imp.content_box.set_parent(self);
        install_viewer_osd_css();
        self.build_tag_popover();

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
        // Ctrl+Scroll → zoom
        // -----------------------------------------------------------------------
        // Capture phase so we see Ctrl+Scroll before the ScrolledWindow
        // consumes the event (which it does whenever there is scrollable overflow).
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
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

        let w = self.downgrade();
        imp.scrolled_window
            .connect_notify_local(Some("width"), move |_, _| {
                if let Some(viewer) = w.upgrade() {
                    viewer.update_picture_zoom();
                }
            });

        let w = self.downgrade();
        imp.scrolled_window
            .connect_notify_local(Some("height"), move |_, _| {
                if let Some(viewer) = w.upgrade() {
                    viewer.update_picture_zoom();
                }
            });

        let viewer_weak = self.downgrade();
        imp.tag_button.connect_clicked(move |_| {
            if let Some(viewer) = viewer_weak.upgrade() {
                viewer.open_tag_popover();
            }
        });

        let viewer_weak = self.downgrade();
        imp.tag_add_button.connect_clicked(move |_| {
            if let Some(viewer) = viewer_weak.upgrade() {
                viewer.open_tag_popover();
            }
        });
    }

    // -----------------------------------------------------------------------
    // Image loading (async via background thread + idle callback)
    // -----------------------------------------------------------------------

    /// Wire the shared preview worker pool to this viewer.
    /// Call once from the window setup, before any images are loaded.
    /// `result_rx` is drained on the GTK main thread; stale results (whose
    /// generation no longer matches `load_gen`) are silently discarded.
    pub fn set_preview_worker(
        &self,
        handle: crate::image_pipeline::worker::PreviewHandle,
        result_rx: async_channel::Receiver<crate::image_pipeline::worker::PreviewResult>,
    ) {
        *self.imp().preview_handle.borrow_mut() = Some(handle);

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
                imp.spinner.stop();
                imp.spinner.set_visible(false);
                match result.image {
                    Ok(crate::image_pipeline::PreviewImage {
                        rgba: bytes,
                        width: w,
                        height: h,
                        ..
                    }) => {
                        crate::bench_event!(
                            "viewer.load.finish",
                            serde_json::json!({
                                "path": result.path.display().to_string(),
                                "source": "decode",
                                "width": w,
                                "height": h,
                            }),
                        );
                        if let Some(ref rc) = *imp.state.borrow() {
                            rc.borrow_mut().library.insert_preview(
                                result.path.clone(),
                                bytes.clone(),
                                w,
                                h,
                            );
                        }
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
                    Err(ref err) => {
                        crate::bench_event!(
                            "viewer.load.fail",
                            serde_json::json!({
                                "path": result.path.display().to_string(),
                                "reason": err.label(),
                            }),
                        );
                        *imp.current_rgba.borrow_mut() = None;
                        imp.picture.set_paintable(None::<&gdk4::Paintable>);
                        let msg = match result.image {
                            Err(crate::image_pipeline::PreviewDecodeError::OpenFailed) =>
                                "Could not open file",
                            Err(crate::image_pipeline::PreviewDecodeError::FormatDetectFailed) |
                            Err(crate::image_pipeline::PreviewDecodeError::Unsupported) =>
                                "Unsupported image format",
                            Err(crate::image_pipeline::PreviewDecodeError::InvalidDimensions) =>
                                "Image has invalid dimensions",
                            _ => "Could not load image",
                        };
                        imp.error_label.set_text(msg);
                        imp.error_label.set_visible(true);
                    }
                }
            }
        });
    }

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
                viewer.refresh_tag_summary();
            }
        });
    }

    /// Register a callback invoked after a successful edit save.
    /// Used by the window to trigger filmstrip thumbnail refresh.
    pub fn set_post_save_callback(&self, cb: impl Fn() + 'static) {
        *self.imp().post_save_cb.borrow_mut() = Some(Box::new(cb));
    }

    /// Clear the viewer (called when the folder changes).
    pub fn clear(&self) {
        let imp = self.imp();
        imp.load_gen.set(imp.load_gen.get().wrapping_add(1));
        imp.tag_popover.popdown();
        self.restore_view_mode();
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();
        imp.tag_osd.set_visible(false);
        self.set_quality_score(None);
        imp.spinner.stop();
        imp.spinner.set_visible(false);
        imp.error_label.set_visible(false);
        self.reset_zoom();
        *imp.current_path.borrow_mut() = None;
        *imp.current_rgba.borrow_mut() = None;
        imp.pending_edit.set(false);
        Self::set_edit_buttons_visible_on(imp, false);
    }

    /// Load and display a full-resolution image from `path`.
    ///
    /// The image is decoded on a background thread using the `image` crate.
    /// Raw RGBA bytes are sent back to the main thread via a one-shot channel,
    /// where a `gdk4::MemoryTexture` is constructed and set on the `GtkPicture`.
    pub fn load_image(&self, path: PathBuf) {
        crate::bench_event!(
            "viewer.load.request",
            serde_json::json!({
                "path": path.display().to_string(),
            }),
        );
        let imp = self.imp();
        let load_gen = imp.load_gen.get().wrapping_add(1);
        imp.load_gen.set(load_gen);
        imp.tag_popover.popdown();
        *imp.current_path.borrow_mut() = Some(path.clone());
        imp.pending_edit.set(false);
        Self::set_edit_buttons_visible_on(imp, false);
        self.restore_view_mode();
        imp.spinner.stop();
        imp.spinner.set_visible(false);
        imp.picture.set_paintable(None::<&gdk4::Paintable>);
        imp.metadata_chip.clear();
        imp.tag_osd.set_visible(false);
        imp.error_label.set_visible(false);
        self.set_quality_score(None);
        *imp.current_rgba.borrow_mut() = None;
        self.refresh_tag_summary();

        // ── Fastest path: use decoded bytes from the preview LRU cache. ────────
        let cached_preview = imp
            .state
            .borrow()
            .as_ref()
            .and_then(|rc| rc.borrow().library.cached_preview(&path));

        if let Some((bytes, w, h)) = cached_preview {
            crate::bench_event!(
                "viewer.load.finish",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "source": "preview_cache",
                    "width": w,
                    "height": h,
                    "duration_ms": 0,
                }),
            );
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
            self.reset_zoom();
            // Metadata loads via the shared worker; result drains in set_metadata_worker loop.
            if let Some(ref handle) = *imp.metadata_handle.borrow() {
                handle.request(path.clone(), load_gen);
            }
            return;
        }

        // ── Fast path: use pre-decoded bytes from prefetch cache. ──────────────
        let prefetched = imp
            .state
            .borrow()
            .as_ref()
            .and_then(|rc| rc.borrow_mut().library.take_prefetch(&path));

        if let Some((bytes, w, h)) = prefetched {
            crate::bench_event!(
                "viewer.load.finish",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "source": "prefetch_cache",
                    "width": w,
                    "height": h,
                    "duration_ms": 0,
                }),
            );
            if let Some(ref rc) = *imp.state.borrow() {
                rc.borrow_mut()
                    .library
                    .insert_preview(path.clone(), bytes.clone(), w, h);
            }
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
            self.reset_zoom();
            // Metadata loads via the shared worker; result drains in set_metadata_worker loop.
            if let Some(ref handle) = *imp.metadata_handle.borrow() {
                handle.request(path.clone(), load_gen);
            }
            return;
        }

        // ── Slow path: submit to the bounded preview worker pool. ──────────────
        imp.spinner.start();
        imp.spinner.set_visible(true);

        // Send the decode request to the shared worker pool. Results are drained
        // by the persistent loop installed in set_preview_worker().
        if let Some(ref handle) = *imp.preview_handle.borrow() {
            handle.request(path.clone(), load_gen);
        }

        // Metadata loads via the shared worker; result drains in set_metadata_worker loop.
        if let Some(ref handle) = *imp.metadata_handle.borrow() {
            handle.request(path.clone(), load_gen);
        }
    }

    // -----------------------------------------------------------------------
    // Zoom
    // -----------------------------------------------------------------------

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

        let base_width = paintable.intrinsic_width().max(1);
        let base_height = paintable.intrinsic_height().max(1);
        let scaled_width = (base_width as f64 * fit_scale * new_zoom).round().max(1.0) as i32;
        let scaled_height = (base_height as f64 * fit_scale * new_zoom).round().max(1.0) as i32;

        imp.picture.set_size_request(scaled_width, scaled_height);

        let scale_ratio = new_zoom / old_zoom;
        self.set_adjustment_value(&hadj, content_focus_x * scale_ratio - focus_x);
        self.set_adjustment_value(&vadj, content_focus_y * scale_ratio - focus_y);
        let pct = (new_zoom * 100.0).round() as u32;
        self.show_zoom_osd(&format!("{pct}%"));
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

    fn fit_scale_for_paintable(&self, paintable: &gdk4::Paintable) -> f64 {
        let base_width = paintable.intrinsic_width().max(1) as f64;
        let base_height = paintable.intrinsic_height().max(1) as f64;
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
            (viewport_width / base_width)
                .min(viewport_height / base_height)
                .min(1.0)
        }
    }

    fn update_picture_zoom(&self) {
        let imp = self.imp();
        let Some(paintable) = imp.picture.paintable() else {
            return;
        };
        if imp.zoom_mode.get() == ZoomMode::Fit && (imp.zoom.get() - 1.0).abs() < f64::EPSILON {
            imp.picture.set_size_request(-1, -1);
            return;
        }

        let fit_scale = self.fit_scale_for_paintable(&paintable);
        let render_scale = (fit_scale * imp.zoom.get()).max(0.01);
        let width = (paintable.intrinsic_width().max(1) as f64 * render_scale)
            .round()
            .max(1.0) as i32;
        let height = (paintable.intrinsic_height().max(1) as f64 * render_scale)
            .round()
            .max(1.0) as i32;
        imp.picture.set_size_request(width, height);
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
            match imp.zoom_mode.get() {
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
            self.refresh_tag_summary();
        } else {
            imp.tag_osd.set_visible(false);
        }
    }

    fn update_quality_indicator(&self, metadata: &crate::metadata::ImageMetadata) {
        self.set_quality_score(Some(&scorer::score_metadata(metadata)));
    }

    fn set_quality_score(&self, quality: Option<&QualityScore>) {
        let imp = self.imp();
        let Some(quality) = quality else {
            imp.metadata_chip.update_quality(None);
            return;
        };

        imp.metadata_chip.update_quality(Some(quality));
    }

    pub fn zoom_mode(&self) -> ZoomMode {
        self.imp().zoom_mode.get()
    }

    pub fn set_zoom_mode(&self, mode: ZoomMode) {
        if self.imp().zoom_mode.get() == mode {
            return;
        }
        self.toggle_zoom_mode();
    }

    /// Return the current image's RGBA pixels, checking the in-memory buffer
    /// first and falling back to the preview LRU cache. Returns `None` when
    /// the image is not in either location (e.g. oversized, evicted, or not yet loaded).
    fn current_rgba_or_cached(&self) -> Option<(Vec<u8>, u32, u32)> {
        let imp = self.imp();
        if let Some(v) = imp.current_rgba.borrow().clone() {
            return Some(v);
        }
        let path = imp.current_path.borrow().clone()?;
        imp.state
            .borrow()
            .as_ref()
            .and_then(|rc| rc.borrow().library.cached_preview(&path))
    }

    /// Apply an in-memory transform to the currently displayed image.
    /// `op` is one of: `"rotate-cw"`, `"rotate-ccw"`, `"flip-h"`, `"flip-v"`.
    /// Falls back to the preview cache when the in-memory buffer is absent.
    pub fn apply_transform(&self, op: &str) {
        use gdk4::{MemoryFormat, MemoryTexture};
        use image::imageops;

        let imp = self.imp();
        // Take the in-memory buffer if available; otherwise load from preview cache.
        let rgba_from_cache = imp.current_rgba.borrow_mut().take();
        let rgba_from_cache = rgba_from_cache.or_else(|| self.current_rgba_or_cached());
        let Some((rgba_bytes, w, h)) = rgba_from_cache else {
            return;
        };
        if w == 0 || h == 0 {
            *imp.current_rgba.borrow_mut() = Some((rgba_bytes, w, h));
            return;
        }

        let Some(buf) = image::RgbaImage::from_raw(w, h, rgba_bytes) else {
            return;
        };

        let transformed = match op {
            "rotate-cw" => image::DynamicImage::ImageRgba8(imageops::rotate90(&buf)),
            "rotate-ccw" => image::DynamicImage::ImageRgba8(imageops::rotate270(&buf)),
            "flip-h" => image::DynamicImage::ImageRgba8(imageops::flip_horizontal(&buf)),
            "flip-v" => image::DynamicImage::ImageRgba8(imageops::flip_vertical(&buf)),
            _ => {
                let rgba_bytes = buf.into_raw();
                *imp.current_rgba.borrow_mut() = Some((rgba_bytes, w, h));
                return;
            }
        };

        let rgba = transformed.into_rgba8();
        let (nw, nh) = (rgba.width(), rgba.height());
        let new_rgba_bytes = rgba.into_raw();
        *imp.current_rgba.borrow_mut() = Some((new_rgba_bytes.clone(), nw, nh));
        let gbytes = glib::Bytes::from_owned(new_rgba_bytes);
        let texture = MemoryTexture::new(
            nw as i32,
            nh as i32,
            MemoryFormat::R8g8b8a8,
            &gbytes,
            (nw * 4) as usize,
        );
        imp.picture
            .set_paintable(Some(texture.upcast_ref::<gdk4::Paintable>()));
        self.reset_zoom();
        self.imp().pending_edit.set(true);
        Self::set_edit_buttons_visible_on(self.imp(), true);
    }

    /// Write the current in-memory texture back to the source file on disk.
    /// JPEG writes are gated because they require lossy re-encoding. PNG and
    /// other lossless outputs save immediately.
    pub fn save_edit(&self) {
        let imp = self.imp();
        let path = match imp.current_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };
        let Some((rgba, w, h)) = self.current_rgba_or_cached() else {
            return;
        };
        if w == 0 || h == 0 {
            return;
        }

        if crate::ui::image_ops::requires_jpeg_reencode_warning(&path) {
            let dialog = libadwaita::AlertDialog::new(
                Some("Save JPEG Edit?"),
                Some("Saving will re-encode this JPEG, which may reduce quality. Save anyway?"),
            );
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("save", "Save");
            dialog.set_default_response(Some("save"));
            dialog.set_close_response("cancel");
            dialog.set_response_appearance("save", libadwaita::ResponseAppearance::Suggested);

            let viewer_weak = self.downgrade();
            dialog.connect_response(None, move |_, response| {
                if response != "save" {
                    return;
                }
                let Some(viewer) = viewer_weak.upgrade() else {
                    return;
                };
                viewer.finish_save_edit(path.clone(), rgba.clone(), w, h);
            });
            let parent_window = self
                .root()
                .and_then(|r| r.downcast::<gtk4::Window>().ok());
            dialog.present(parent_window.as_ref());
            return;
        }

        self.finish_save_edit(path, rgba, w, h);
    }

    /// Reload the original file from disk, discarding the in-memory transform.
    pub fn discard_edit(&self) {
        let path = self.imp().current_path.borrow().clone();
        if let Some(p) = path {
            self.load_image(p); // load_image clears pending_edit and hides buttons
        }
    }

    fn finish_save_edit(&self, path: PathBuf, rgba: Vec<u8>, w: u32, h: u32) {
        let imp = self.imp();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let result = crate::ui::image_ops::save_edit_pixels(&path, &ext, &rgba, w, h);

        if let Ok(saved_path) = result {
            if let Some(ref rc) = *imp.state.borrow() {
                rc.borrow_mut().library.invalidate_path_caches(&saved_path);
            }
            imp.pending_edit.set(false);
            Self::set_edit_buttons_visible_on(imp, false);
            if let Some(ref cb) = *imp.post_save_cb.borrow() {
                cb();
            }
            self.load_image(saved_path);
        } else {
            eprintln!(
                "save_edit: failed to write {}: {}",
                path.display(),
                result.err().unwrap_or_else(|| "unknown error".to_string())
            );
        }
    }

    // -----------------------------------------------------------------------
    // Metadata overlay
    // -----------------------------------------------------------------------

    pub fn toggle_metadata(&self) {
        self.set_metadata_visible(!self.metadata_visible());
    }

    pub fn open_tag_popover(&self) {
        let imp = self.imp();
        let path = match imp.current_path.borrow().clone() {
            Some(path) => path,
            None => return,
        };
        let db = imp
            .state
            .borrow()
            .as_ref()
            .and_then(|state| state.borrow().tags.clone());
        if db.is_none() {
            return;
        }

        {
            let mut tag_state = imp.tag_state.borrow_mut();
            tag_state.db = db;
            tag_state.path = Some(path);
        }

        imp.tag_entry.set_text("");
        self.refresh_tag_chips();
        self.refresh_tag_summary();
        imp.tag_popover.popup();
    }

    pub fn show_smart_tag_btn(&self) {
        self.imp().smart_tag_btn.set_visible(true);
    }

    fn build_tag_popover(&self) {
        let imp = self.imp();

        imp.tag_entry.set_placeholder_text(Some("Add tag"));
        imp.tag_entry.set_hexpand(true);

        imp.tag_flowbox
            .set_selection_mode(gtk4::SelectionMode::None);
        imp.tag_flowbox.set_row_spacing(6);
        imp.tag_flowbox.set_column_spacing(6);
        imp.tag_flowbox.set_max_children_per_line(8);
        imp.tag_flowbox.set_homogeneous(false);
        imp.tag_flowbox.set_valign(gtk4::Align::Start);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_size_request(320, -1);

        let header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let title = gtk4::Label::new(Some("Tags"));
        title.set_halign(gtk4::Align::Start);
        title.set_hexpand(true);
        title.add_css_class("heading");
        header_row.append(&title);
        header_row.append(&imp.smart_tag_spinner);
        header_row.append(&imp.smart_tag_btn);

        let chips_scroll = gtk4::ScrolledWindow::new();
        chips_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        chips_scroll.set_min_content_height(72);
        chips_scroll.set_max_content_height(180);
        chips_scroll.set_child(Some(&imp.tag_flowbox));

        content.append(&header_row);
        content.append(&imp.tag_entry);

        let suggestions_label = gtk4::Label::new(Some("Suggestions"));
        suggestions_label.set_halign(gtk4::Align::Start);
        suggestions_label.add_css_class("dim-label");
        imp.suggestions_box.append(&suggestions_label);
        imp.suggestions_box.append(&imp.suggestions_flow);
        imp.suggestions_box.append(&imp.suggestions_add_all);
        content.append(&imp.suggestions_box);

        content.append(&chips_scroll);

        imp.tag_popover.set_child(Some(&content));
        imp.tag_popover.set_parent(&imp.tag_anchor);

        let entry = imp.tag_entry.clone();
        imp.tag_popover.connect_show(move |_| {
            entry.grab_focus();
        });

        let sugg_box = imp.suggestions_box.clone();
        let sugg_flow = imp.suggestions_flow.clone();
        let sugg_add_all = imp.suggestions_add_all.clone();
        imp.tag_popover.connect_closed(move |_| {
            while let Some(child) = sugg_flow.first_child() {
                sugg_flow.remove(&child);
            }
            sugg_add_all.set_visible(false);
            sugg_box.set_visible(false);
        });

        let key = gtk4::EventControllerKey::new();
        let popover = imp.tag_popover.clone();
        key.connect_key_pressed(move |_, key, _, _| {
            if key == gdk4::Key::Escape {
                popover.popdown();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        imp.tag_entry.add_controller(key);

        let viewer_weak = self.downgrade();
        imp.tag_entry.connect_activate(move |entry| {
            let Some(viewer) = viewer_weak.upgrade() else {
                return;
            };
            let text = entry.text();
            let tag = text.trim();
            if tag.is_empty() {
                return;
            }
            let state = viewer.imp().tag_state.borrow();
            let (Some(db), Some(path)) = (state.db.clone(), state.path.clone()) else {
                return;
            };
            db.add_tag(&path, tag);
            entry.set_text("");
            drop(state);
            viewer.refresh_tag_chips();
            viewer.refresh_tag_summary();
        });

        let viewer_weak = self.downgrade();
        imp.smart_tag_btn.connect_clicked(move |btn| {
            let Some(viewer) = viewer_weak.upgrade() else {
                return;
            };
            let imp = viewer.imp();

            let Some((rgba, w, h)) = viewer.current_rgba_or_cached() else {
                return;
            };

            let tagger = imp
                .state
                .borrow()
                .as_ref()
                .and_then(|s| s.borrow().smart_tagger.clone());
            let Some(tagger) = tagger else {
                return;
            };

            let db_path = {
                let state = imp.tag_state.borrow();
                state.db.clone().zip(state.path.clone())
            };
            let Some((db, path)) = db_path else {
                return;
            };

            btn.set_visible(false);
            imp.smart_tag_spinner.set_spinning(true);
            imp.smart_tag_spinner.set_visible(true);
            while let Some(child) = imp.suggestions_flow.first_child() {
                imp.suggestions_flow.remove(&child);
            }
            imp.suggestions_add_all.set_visible(false);
            imp.suggestions_box.set_visible(false);

            let (tx, rx) = async_channel::bounded::<Vec<String>>(1);
            std::thread::spawn(move || {
                let tags = tagger.suggest_tags(&rgba, w, h);
                tx.send_blocking(tags).ok();
            });

            let viewer_weak2 = viewer.downgrade();
            let db2 = db.clone();
            let path2 = path.clone();
            glib::MainContext::default().spawn_local(async move {
                let Ok(tags) = rx.recv().await else {
                    return;
                };
                let Some(viewer) = viewer_weak2.upgrade() else {
                    return;
                };
                let imp = viewer.imp();

                imp.smart_tag_spinner.set_spinning(false);
                imp.smart_tag_spinner.set_visible(false);
                imp.smart_tag_btn.set_visible(true);

                if tags.is_empty() {
                    return;
                }

                for tag in &tags {
                    let chip = gtk4::Button::with_label(tag);
                    chip.add_css_class("flat");
                    chip.add_css_class("pill");
                    chip.add_css_class("suggested-tag-chip");

                    let db3 = db2.clone();
                    let path3 = path2.clone();
                    let tag3 = tag.clone();
                    let viewer_weak3 = viewer.downgrade();
                    chip.connect_clicked(move |chip| {
                        db3.add_tag(&path3, &tag3);
                        if let Some(parent) = chip.parent() {
                            if let Some(flow_child) = parent.parent() {
                                if let Ok(fc) = flow_child.downcast::<gtk4::FlowBoxChild>() {
                                    if let Some(flow) = fc.parent() {
                                        if let Ok(fb) = flow.downcast::<gtk4::FlowBox>() {
                                            fb.remove(&fc);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(viewer) = viewer_weak3.upgrade() {
                            viewer.refresh_tag_summary();
                        }
                    });

                    let child = gtk4::FlowBoxChild::new();
                    child.set_child(Some(&chip));
                    imp.suggestions_flow.insert(&child, -1);
                }

                imp.suggestions_add_all.set_visible(true);
                imp.suggestions_box.set_visible(true);
            });
        });

        let viewer_weak = self.downgrade();
        imp.suggestions_add_all.connect_clicked(move |_| {
            let Some(viewer) = viewer_weak.upgrade() else {
                return;
            };
            let imp = viewer.imp();
            let state = imp.tag_state.borrow();
            let (Some(db), Some(path)) = (state.db.clone(), state.path.clone()) else {
                return;
            };
            drop(state);

            let mut child = imp.suggestions_flow.first_child();
            while let Some(flow_child) = child {
                child = flow_child.next_sibling();
                if let Ok(fc) = flow_child.downcast::<gtk4::FlowBoxChild>() {
                    if let Some(btn) = fc.child().and_then(|w| w.downcast::<gtk4::Button>().ok()) {
                        db.add_tag(&path, &btn.label().unwrap_or_default());
                    }
                    imp.suggestions_flow.remove(&fc);
                }
            }

            imp.suggestions_add_all.set_visible(false);
            imp.suggestions_box.set_visible(false);
            viewer.refresh_tag_summary();
        });
    }

    fn refresh_tag_chips(&self) {
        let imp = self.imp();
        while let Some(child) = imp.tag_flowbox.first_child() {
            imp.tag_flowbox.remove(&child);
        }

        let state = imp.tag_state.borrow();
        let (Some(db), Some(path)) = (state.db.clone(), state.path.clone()) else {
            return;
        };
        drop(state);

        for tag in db.tags_for_path(&path) {
            let chip = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            chip.add_css_class("pill");

            let label = gtk4::Label::new(Some(&tag));
            label.set_halign(gtk4::Align::Start);

            let remove = gtk4::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("flat");
            remove.set_focus_on_click(false);
            remove.set_tooltip_text(Some("Remove tag"));

            chip.append(&label);
            chip.append(&remove);
            imp.tag_flowbox.insert(&chip, -1);

            let viewer_weak = self.downgrade();
            let tag_name = tag.clone();
            remove.connect_clicked(move |_| {
                let Some(viewer) = viewer_weak.upgrade() else {
                    return;
                };
                let state = viewer.imp().tag_state.borrow();
                let (Some(db), Some(path)) = (state.db.clone(), state.path.clone()) else {
                    return;
                };
                db.remove_tag(&path, &tag_name);
                drop(state);
                viewer.refresh_tag_chips();
                viewer.refresh_tag_summary();
            });
        }
    }

    fn refresh_tag_summary(&self) {
        let imp = self.imp();
        let path = match imp.current_path.borrow().clone() {
            Some(path) => path,
            None => {
                imp.tag_osd.set_visible(false);
                return;
            }
        };

        let db = imp
            .state
            .borrow()
            .as_ref()
            .and_then(|state| state.borrow().tags.clone());
        let Some(db) = db else {
            imp.tag_osd.set_visible(false);
            return;
        };

        let tags = db.tags_for_path(&path);
        let summary = match tags.len() {
            0 => "Add tag".to_string(),
            1..=3 => tags.join(" · "),
            _ => format!("{} +{}", tags[..3].join(" · "), tags.len() - 3),
        };
        let tooltip = if tags.is_empty() {
            "Add tags".to_string()
        } else {
            format!("Tags: {}", tags.join(", "))
        };

        imp.tag_label.set_text(&summary);
        imp.tag_button.set_tooltip_text(Some(&tooltip));
        imp.tag_osd.set_visible(imp.metadata_visible.get());
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
    pub fn start_upscale(
        &self,
        path: PathBuf,
        scale: u32,
        model: crate::upscale::UpscaleModel,
        trigger_btn: gtk4::Button,
    ) {
        use crate::model::ImageEntry;
        use crate::upscale::backend::UpscaleBackend;
        use crate::upscale::backends::cli::CliBackend;
        use crate::upscale::backends::onnx::OnnxBackend;
        use crate::upscale::runner::{UpscaleEvent, UpscaleRunner};
        use crate::upscale::{
            ComfyUiBackend, OnnxUpscaleModel, UpscaleBackendKind, UpscaleCompressionMode,
            UpscaleJobConfig, UpscaleOutputFormat,
        };

        let imp = self.imp();

        let (binary, settings, mut selected_dims, ops_queue) = {
            let st = imp.state.borrow();
            let Some(ref rc) = *st else {
                trigger_btn.set_sensitive(true);
                return;
            };
            let state = rc.borrow();
            (
                state.upscale_binary.clone(),
                state.settings.clone(),
                state
                    .library
                    .selected_entry()
                    .and_then(|e: ImageEntry| e.dimensions())
                    .unwrap_or((0, 0)),
                state.ops.clone(),
            )
        };

        let backend_kind = UpscaleBackendKind::from_settings(&settings.upscale_backend);
        let backend: Box<dyn UpscaleBackend> = match backend_kind {
            UpscaleBackendKind::Cli => {
                let Some(bin) = binary else {
                    trigger_btn.set_sensitive(true);
                    return;
                };
                Box::new(CliBackend::new(bin))
            }
            UpscaleBackendKind::Onnx => Box::new(OnnxBackend::new(
                OnnxUpscaleModel::from_settings(&settings.onnx_upscale_model),
            )),
            UpscaleBackendKind::ComfyUi => Box::new(ComfyUiBackend::new(&settings.comfyui_url)),
        };
        if selected_dims == (0, 0) {
            let meta = crate::metadata::ImageMetadata::load(&path);
            if meta.width > 0 && meta.height > 0 {
                selected_dims = (meta.width, meta.height);
            }
        }

        let requested_scale = {
            if scale == 0 {
                let (w, h) = selected_dims;
                UpscaleRunner::smart_scale(w, h)
            } else {
                scale
            }
        };
        let output_format = UpscaleRunner::select_output_format(
            &path,
            UpscaleOutputFormat::from_settings(&settings.upscaler_output_format),
            UpscaleCompressionMode::from_settings(&settings.upscaler_compression_mode),
        );
        let final_output = output_path_for_upscale(&path, output_format);
        let job = UpscaleJobConfig {
            source_dimensions: selected_dims,
            requested_scale,
            execution_scale: model.native_scale(),
            model,
            output_format,
            compression_mode: UpscaleCompressionMode::from_settings(
                &settings.upscaler_compression_mode,
            ),
            quality: settings.upscaler_quality.clamp(50, 100) as u8,
            tile_size: (settings.upscaler_tile_size > 0)
                .then_some(settings.upscaler_tile_size as u32),
            gpu_id: (settings.upscaler_gpu_id >= 0).then_some(settings.upscaler_gpu_id as u32),
        };

        let rx = if let Some(dir) = final_output.parent() {
            match std::fs::create_dir_all(dir) {
                Ok(()) => {
                    let output = pending_output_path(&final_output);
                    backend.run(path.clone(), output, job)
                }
                Err(err) => {
                    let (tx, rx) = async_channel::bounded(1);
                    let _ = tx.try_send(UpscaleEvent::Failed(format!(
                        "Failed to create output directory {}: {err}",
                        dir.display()
                    )));
                    rx
                }
            }
        } else {
            let output = pending_output_path(&final_output);
            backend.run(path.clone(), output, job)
        };

        let widget_weak = self.downgrade();
        let op = ops_queue.add("Upscaling image");
        // Keep a strong ref to trigger_btn inside the closure so we can
        // re-enable it on completion. The weak-ref pattern caused the strong
        // ref to drop when start_upscale() returned, leaving upgrade() returning None.

        glib::MainContext::default().spawn_local(async move {
            let mut op = Some(op);
            while let Ok(event) = rx.recv().await {
                let Some(viewer) = widget_weak.upgrade() else {
                    break;
                };
                match event {
                    UpscaleEvent::Progress(Some(f)) => {
                        if let Some(op) = op.as_ref() {
                            op.progress(Some(f));
                        }
                    }
                    UpscaleEvent::Progress(None) => {
                        if let Some(op) = op.as_ref() {
                            op.progress(None);
                        }
                    }
                    UpscaleEvent::Done(out_path) => {
                        trigger_btn.set_sensitive(true);
                        let viewer_weak = viewer.downgrade();
                        *viewer.imp().pending_commit_fn.borrow_mut() =
                            Some(Box::new(move |pending_path: PathBuf| {
                                let Some(v) = viewer_weak.upgrade() else {
                                    return;
                                };
                                let final_path = committed_output_path(&pending_path);
                                if final_path != pending_path
                                    && std::fs::rename(&pending_path, &final_path).is_ok()
                                {
                                    v.insert_committed_output(&final_path);
                                    v.load_image(final_path);
                                    return;
                                }
                                v.insert_committed_output(&pending_path);
                                v.load_image(pending_path);
                            }));
                        viewer.show_comparison(path.clone(), out_path);
                        if let Some(op) = op.take() {
                            op.complete();
                        }
                        break;
                    }
                    UpscaleEvent::Failed(msg) => {
                        let vimp = viewer.imp();
                        vimp.progress_bar.set_visible(false);
                        trigger_btn.set_sensitive(true);
                        let dialog =
                            libadwaita::AlertDialog::new(Some("Upscale Failed"), Some(&msg));
                        dialog.add_response("ok", "OK");
                        if let Some(root) = viewer.root() {
                            if let Ok(window) = root.downcast::<gtk4::Window>() {
                                dialog.present(Some(&window));
                            }
                        }
                        eprintln!("Upscale failed: {msg}");
                        if let Some(op) = op.take() {
                            op.fail(msg);
                        }
                        break;
                    }
                }
            }
        });
    }

    /// Export `source` to a temp file in the destination folder, then show
    /// the before/after comparison. `on_commit` is called with the temp path
    /// when the user clicks Commit; `discard_convert` deletes the temp file.
    pub fn start_downscale_preview(
        &self,
        source: PathBuf,
        config: crate::export::ExportConfig,
        on_commit: Box<dyn FnOnce(PathBuf) + 'static>,
    ) {
        let imp = self.imp();
        let ops_queue = {
            let st = imp.state.borrow();
            let Some(ref rc) = *st else { return };
            let ops = rc.borrow().ops.clone();
            ops
        };

        let stem = source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let ext = crate::export::format_extension(config.format);
        let pid = std::process::id();
        let temp_path = config.destination.join(format!("{stem}.pending-{pid}.{ext}"));

        *imp.pending_output.borrow_mut() = Some(temp_path.clone());
        *imp.pending_commit_fn.borrow_mut() = Some(on_commit);

        let source_c = source.clone();
        let temp_c = temp_path.clone();
        let max_edge = config.max_edge;
        let format = config.format;
        let quality = config.quality;

        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        rayon::spawn(move || {
            let result =
                crate::export::export_to_path(&source_c, &temp_c, max_edge, format, quality)
                    .map_err(|e| e.to_string());
            let _ = tx.send_blocking(result);
        });

        let widget_weak = self.downgrade();
        let op = ops_queue.add("Preparing preview");
        glib::MainContext::default().spawn_local(async move {
            if let Ok(result) = rx.recv().await {
                op.complete();
                let Some(viewer) = widget_weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(()) => {
                        viewer.show_comparison(source, temp_path);
                    }
                    Err(msg) => {
                        let vimp = viewer.imp();
                        vimp.pending_output.borrow_mut().take();
                        vimp.pending_commit_fn.borrow_mut().take();
                        let dialog =
                            libadwaita::AlertDialog::new(Some("Export Failed"), Some(&msg));
                        dialog.add_response("ok", "OK");
                        if let Some(root) = viewer.root() {
                            if let Ok(win) = root.downcast::<gtk4::Window>() {
                                dialog.present(Some(&win));
                            }
                        }
                    }
                }
            }
        });
    }

    /// Load both images into the comparison widget and switch the stack to the
    /// "compare" page. `show_actions` controls whether Save/Discard are shown.
    fn show_comparison_with_actions(
        &self,
        before_path: PathBuf,
        after_path: PathBuf,
        show_actions: bool,
    ) {
        let imp = self.imp();
        imp.comparison.reset_zoom();
        *imp.pending_output.borrow_mut() = show_actions.then_some(after_path.clone());
        imp.stack.set_visible_child_name("compare");
        let viewer_weak = self.downgrade();
        imp.comparison.load(before_path, after_path, move || {
            let Some(viewer) = viewer_weak.upgrade() else {
                return;
            };
            let imp = viewer.imp();
            imp.progress_bar.set_visible(false);
            viewer.set_comparison_buttons_visible(show_actions);
            imp.stack.set_visible_child_name("compare");
        });
    }

    /// Load both images into the comparison widget, show Commit/Discard, and
    /// switch the stack to the "compare" page.
    fn show_comparison(&self, before_path: PathBuf, after_path: PathBuf) {
        self.show_comparison_with_actions(before_path, after_path, true);
    }

    pub fn toggle_debug_comparison(&self) {
        let imp = self.imp();
        if imp.stack.visible_child_name().as_deref() == Some("compare")
            && imp.pending_output.borrow().is_none()
        {
            self.restore_view_mode();
            return;
        }

        let path = imp.current_path.borrow().clone().or_else(|| {
            imp.state.borrow().as_ref().and_then(|state| {
                state
                    .borrow()
                    .library
                    .selected_entry()
                    .map(|entry| entry.path())
            })
        });

        let Some(path) = path else {
            return;
        };

        self.show_comparison_with_actions(path.clone(), path, false);
    }

    /// Generic commit: runs the pending commit closure (set by start_upscale or
    /// start_downscale_preview) with the temp output path, then restores the view.
    pub fn commit_convert(&self) {
        let imp = self.imp();
        let pending_path = imp.pending_output.borrow_mut().take();
        let commit_fn = imp.pending_commit_fn.borrow_mut().take();
        self.restore_view_mode();
        if let (Some(path), Some(f)) = (pending_path, commit_fn) {
            f(path);
        }
    }

    /// Generic discard: deletes the temp output file and restores the view.
    pub fn discard_convert(&self) {
        let imp = self.imp();
        let out_path = imp.pending_output.borrow_mut().take();
        imp.pending_commit_fn.borrow_mut().take();
        if let Some(path) = out_path {
            let _ = std::fs::remove_file(&path);
        }
        self.restore_view_mode();
    }

    /// Commit: load the upscaled output into the viewer and return to the
    /// normal view. Does NOT copy the file — the output path IS the final
    /// location (`<src_dir>/upscaled/<name>`).
    pub fn commit_upscale(&self) {
        self.commit_convert();
    }

    /// Discard: delete the temp output file and return to the normal viewer.
    pub fn discard_upscale(&self) {
        self.discard_convert();
    }

    fn restore_view_mode(&self) {
        let imp = self.imp();
        *imp.pending_output.borrow_mut() = None;
        imp.comparison.clear();
        self.set_comparison_buttons_visible(false);
        imp.stack.set_visible_child_name("view");
    }

    fn insert_committed_output(&self, path: &std::path::Path) {
        let Some(state) = self.imp().state.borrow().as_ref().cloned() else {
            return;
        };
        let mut state = state.borrow_mut();
        if let Some(index) = state.library.insert_path(path.to_path_buf()) {
            state.library.selected_index = Some(index);
        }
    }
}

fn output_path_for_upscale(
    input_path: &std::path::Path,
    format: crate::upscale::UpscaleOutputFormat,
) -> PathBuf {
    let parent = input_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("upscaled");
    parent
        .join("upscaled")
        .join(format!("{stem}.{}", format.extension()))
}

fn pending_output_path(final_output: &std::path::Path) -> PathBuf {
    let stem = final_output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("upscaled");
    let ext = final_output
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
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
    let base = pending_output.with_file_name(&trimmed);
    if !base.exists() {
        return base;
    }
    // File already exists — find a free slot: stem_1.ext, stem_2.ext, …
    let dir = pending_output
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let trimmed_path = std::path::Path::new(&trimmed);
    let stem = trimmed_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("upscaled");
    let ext = trimmed_path.extension().and_then(|s| s.to_str());
    for i in 1u32.. {
        let candidate = match ext {
            Some(e) => dir.join(format!("{stem}_{i}.{e}")),
            None => dir.join(format!("{stem}_{i}")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    base
}

fn install_viewer_osd_css() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "
            .tag-osd {
                padding: 8px 10px;
                border-radius: 16px;
                background-color: rgba(28, 28, 30, 0.72);
                box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
            }
            .tag-osd-pill,
            .tag-osd-add {
                border-radius: 999px;
                min-height: 28px;
                background-color: rgba(255, 255, 255, 0.08);
                color: white;
            }
            .tag-osd-pill {
                padding: 0 10px;
            }
            .tag-osd-pill:hover,
            .tag-osd-add:hover {
                background-color: rgba(255, 255, 255, 0.14);
            }
            .tag-osd-add {
                min-width: 28px;
                padding: 0;
            }
            .zoom-osd {
                padding: 10px 18px;
                border-radius: 18px;
                background-color: rgba(28, 28, 30, 0.78);
                box-shadow: 0 8px 22px rgba(0, 0, 0, 0.22);
            }
            ",
        );
        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}
