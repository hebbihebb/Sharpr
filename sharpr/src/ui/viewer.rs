use gtk4::gio;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

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
        pub image_menu: gtk4::PopoverMenu,
        pub metadata_chip: MetadataChip,
        pub spinner: gtk4::Spinner,
        /// Shown briefly when a decode error prevents the image from loading.
        pub error_label: gtk4::Label,
        pub zoom_label: gtk4::Label,
        /// D36: 16:9 guide drawn over the displayed image. It takes no
        /// input itself; a capture gesture on the overlay grabs it (D38).
        pub guide: crate::ui::guide_layer::GuideLayer,
        pub guide_on: Cell<bool>,
        /// Crop Copy covers the viewer; scrolling is its to use.
        pub crop_open: Cell<bool>,
        /// Where the user moved or resized the guide, in overlay coordinates;
        /// `None` fits it to the image as shown at Fit.
        pub(crate) guide_manual: Cell<Option<super::Rect>>,
        pub(crate) guide_drag: Cell<Option<(super::GuideGrab, super::Rect)>>,
        pub zoom_hide_source: RefCell<Option<glib::SourceId>>,
        /// Path of the image currently displayed — set by load_image(), cleared by clear().
        pub current_path: RefCell<Option<PathBuf>>,
        /// Zoom/Fit toggle button in the header — stored so mode changes can
        /// update the icon without an extra signal.
        pub zoom_btn: std::cell::RefCell<Option<gtk4::Button>>,
        pub zoom: Cell<f64>,
        pub zoom_mode: Cell<super::ZoomMode>,
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
        /// Info's close-images count (item 29): the searcher for the stored
        /// vectors, the image counted (or being counted), and a generation
        /// that cancels a count once the image changes.
        pub visual_searcher: RefCell<Option<crate::visual_search::search::Searcher>>,
        pub close_counted: RefCell<Option<PathBuf>>,
        pub close_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
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

            let guide = crate::ui::guide_layer::GuideLayer::default();
            guide.set_can_target(false);
            guide.set_visible(false);

            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(&scrolled_window));
            overlay.add_overlay(&guide);
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
                image_menu: gtk4::PopoverMenu::from_model(None::<&gio::MenuModel>),
                metadata_chip,
                spinner,
                error_label,
                zoom_label,
                guide,
                guide_on: Cell::new(false),
                crop_open: Cell::new(false),
                guide_manual: Cell::new(None),
                guide_drag: Cell::new(None),
                zoom_hide_source: RefCell::new(None),
                current_path: RefCell::new(None),
                zoom_btn: std::cell::RefCell::new(None),
                zoom: Cell::new(1.0),
                zoom_mode: Cell::new(super::ZoomMode::Fit),
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
                visual_searcher: RefCell::new(None),
                close_counted: RefCell::new(None),
                close_gen: Default::default(),
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
            if self.image_menu.parent().is_some() {
                self.image_menu.unparent();
            }
            self.content_box.unparent();
        }
    }

    impl WidgetImpl for ViewerPane {}
}

/// The largest 16:9 rectangle centred inside an image occupying
/// `x, y, width, height` (D36). A wider image overhangs the result at its
/// sides, a taller one above and below; an exact 16:9 image matches it.
pub(crate) fn guide_rect(x: f64, y: f64, width: f64, height: f64) -> Option<(f64, f64, f64, f64)> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let (w, h) = if width * 9.0 > height * 16.0 {
        (height * 16.0 / 9.0, height)
    } else {
        (width, width * 9.0 / 16.0)
    };
    Some((x + (width - w) / 2.0, y + (height - h) / 2.0, w, h))
}

/// A rectangle: x, y, width, height.
pub(crate) type Rect = (f64, f64, f64, f64);

/// How close to the guide's corner or outline, in screen pixels, a press
/// grabs it (D38).
const GUIDE_GRAB: f64 = 12.0;
/// The narrowest the guide can be resized to.
const GUIDE_MIN_WIDTH: f64 = 96.0;

/// What a press on the guide grabbed: a corner resizes, the outline moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuideGrab {
    Corner(crate::export::Corner),
    Edge,
}

/// The part of the guide a press at (`px`, `py`) grabs, if any. Presses
/// inside, away from the outline, are left to panning.
pub(crate) fn guide_grab((x, y, w, h): Rect, px: f64, py: f64) -> Option<GuideGrab> {
    use crate::export::Corner;
    let near = |a: f64, b: f64| (a - b).abs() <= GUIDE_GRAB;
    let corner = match (near(px, x), near(px, x + w), near(py, y), near(py, y + h)) {
        (true, _, true, _) => Some(Corner::TopLeft),
        (_, true, true, _) => Some(Corner::TopRight),
        (true, _, _, true) => Some(Corner::BottomLeft),
        (_, true, _, true) => Some(Corner::BottomRight),
        _ => None,
    };
    if let Some(corner) = corner {
        return Some(GuideGrab::Corner(corner));
    }
    let within =
        |p: f64, start: f64, len: f64| p >= start - GUIDE_GRAB && p <= start + len + GUIDE_GRAB;
    let on_edge = (within(px, x, w) && (near(py, y) || near(py, y + h)))
        || (within(py, y, h) && (near(px, x) || near(px, x + w)));
    on_edge.then_some(GuideGrab::Edge)
}

/// `rect` kept inside `view`: shrunk about its centre (still 16:9) when too
/// big, then shifted in.
pub(crate) fn guide_within((x, y, w, h): Rect, (vx, vy, vw, vh): Rect) -> Rect {
    let nw = w.min(vw).min(vh * 16.0 / 9.0).max(0.0);
    let nh = if nw < w { nw * 9.0 / 16.0 } else { h };
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let clamp = |v: f64, lo: f64, hi: f64| v.max(lo).min(hi.max(lo));
    (
        clamp(cx - nw / 2.0, vx, vx + vw - nw),
        clamp(cy - nh / 2.0, vy, vy + vh - nh),
        nw,
        nh,
    )
}

/// `rect` moved by (`dx`, `dy`), kept inside `view`.
pub(crate) fn guide_moved((x, y, w, h): Rect, dx: f64, dy: f64, view: Rect) -> Rect {
    guide_within((x + dx, y + dy, w, h), view)
}

/// `rect` resized by dragging `corner` by (`dx`, `dy`): the opposite corner
/// stays put, the drag along the guide's diagonal decides the size, 16:9
/// holds, and the result stays inside `view`.
pub(crate) fn guide_resized(
    (x, y, w, h): Rect,
    corner: crate::export::Corner,
    dx: f64,
    dy: f64,
    (vx, vy, vw, vh): Rect,
) -> Rect {
    use crate::export::Corner;
    let (anchor_x, anchor_y, grow_x, grow_y) = match corner {
        Corner::TopLeft => (x + w, y + h, -1.0, -1.0),
        Corner::TopRight => (x, y + h, 1.0, -1.0),
        Corner::BottomLeft => (x + w, y, -1.0, 1.0),
        Corner::BottomRight => (x, y, 1.0, 1.0),
    };
    let room_x = if grow_x > 0.0 {
        vx + vw - anchor_x
    } else {
        anchor_x - vx
    };
    let room_y = if grow_y > 0.0 {
        vy + vh - anchor_y
    } else {
        anchor_y - vy
    };
    let most = room_x.min(room_y * 16.0 / 9.0).max(0.0);
    // Follow the drag projected onto the guide's diagonal, so the size
    // changes smoothly in every direction.
    let units = (dx * grow_x * 16.0 + dy * grow_y * 9.0) / (16.0 * 16.0 + 9.0 * 9.0);
    let width = w + units * 16.0;
    let width = width.clamp(GUIDE_MIN_WIDTH.min(most), most);
    let height = width * 9.0 / 16.0;
    (
        if grow_x > 0.0 {
            anchor_x
        } else {
            anchor_x - width
        },
        if grow_y > 0.0 {
            anchor_y
        } else {
            anchor_y - height
        },
        width,
        height,
    )
}

/// The guide's share of the displayed `image`, as fractions of its width
/// and height (x, y, width, height), or `None` when they don't overlap.
pub(crate) fn guide_frame((gx, gy, gw, gh): Rect, (ix, iy, iw, ih): Rect) -> Option<Rect> {
    if iw <= 0.0 || ih <= 0.0 {
        return None;
    }
    let left = gx.max(ix);
    let top = gy.max(iy);
    let right = (gx + gw).min(ix + iw);
    let bottom = (gy + gh).min(iy + ih);
    (right > left && bottom > top).then(|| {
        (
            (left - ix) / iw,
            (top - iy) / ih,
            (right - left) / iw,
            (bottom - top) / ih,
        )
    })
}

fn bounds_rect(bounds: &gtk4::graphene::Rect) -> Rect {
    (
        f64::from(bounds.x()),
        f64::from(bounds.y()),
        f64::from(bounds.width()),
        f64::from(bounds.height()),
    )
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
        let weak = widget.downgrade();
        widget.imp().metadata_chip.connect_info_opened(move || {
            if let Some(viewer) = weak.upgrade() {
                viewer.refresh_close_count();
            }
        });
        widget
    }

    /// Visual search's searcher, for Info's close-images count.
    pub fn set_visual_searcher(&self, searcher: Option<crate::visual_search::search::Searcher>) {
        *self.imp().visual_searcher.borrow_mut() = searcher;
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
                if imp.chip_hovered.get() || imp.metadata_chip.is_info_open() {
                    return;
                }
                imp.metadata_chip.add_css_class("sharpr-idle");
                imp.metadata_chip.set_can_target(false);
            });
        *imp.chip_idle_source.borrow_mut() = Some(source);
    }

    /// D36: shows or hides the 16:9 guide; it changes no pixels or files.
    /// Showing it again fits it to the image, undoing any move (D38).
    pub fn toggle_guide(&self) {
        let imp = self.imp();
        let on = !imp.guide_on.get();
        imp.guide_on.set(on);
        imp.guide_manual.set(None);
        imp.guide_drag.set(None);
        imp.guide.set_visible(on);
        imp.guide.queue_draw();
    }

    /// The guide and the viewport, in overlay coordinates. Until the user
    /// moves it, the guide fits the image as shown at Fit, so zooming and
    /// panning leave it in place on screen (D38).
    pub(crate) fn guide_geometry(&self) -> Option<(Rect, Rect)> {
        let imp = self.imp();
        let paintable = imp.picture.paintable()?;
        let view = bounds_rect(&imp.scrolled_window.compute_bounds(&imp.overlay)?);
        let guide = match imp.guide_manual.get() {
            Some(rect) => guide_within(rect, view),
            None => {
                let (vx, vy, vw, vh) = view;
                let (bw, bh) = self.image_size(&paintable);
                let scale = (vw / bw).min(vh / bh);
                let (fw, fh) = (bw * scale, bh * scale);
                guide_rect(vx + (vw - fw) / 2.0, vy + (vh - fh) / 2.0, fw, fh)?
            }
        };
        Some((guide, view))
    }

    /// The part of the guide a press at overlay point (`x`, `y`) grabs.
    pub(crate) fn guide_grab_at(&self, x: f64, y: f64) -> Option<(GuideGrab, Rect)> {
        let imp = self.imp();
        if !imp.guide_on.get() {
            return None;
        }
        // The corner chip stays clickable where it covers the guide.
        let chip = imp.metadata_chip.upcast_ref::<gtk4::Widget>();
        if imp
            .overlay
            .pick(x, y, gtk4::PickFlags::DEFAULT)
            .is_some_and(|w| &w == chip || w.is_ancestor(chip))
        {
            return None;
        }
        let (guide, _) = self.guide_geometry()?;
        guide_grab(guide, x, y).map(|grab| (grab, guide))
    }

    /// The guide's share of the displayed image while it is shown, for
    /// Crop Copy to start from (D38).
    pub fn guide_frame(&self) -> Option<Rect> {
        let imp = self.imp();
        if !imp.guide_on.get() {
            return None;
        }
        let (guide, _) = self.guide_geometry()?;
        let image = bounds_rect(&imp.picture.compute_bounds(&imp.overlay)?);
        guide_frame(guide, image)
    }

    /// The image's scale on screen (pixels per image pixel) and the image
    /// point at the middle of the viewport as fractions of the image, for
    /// Crop Copy to open on the same view (the selection then lands where
    /// the guide was). Read from the picture's actual size and place, so it
    /// holds even while the scroll adjustments catch up with a zoom.
    pub fn view_state(&self) -> Option<(f64, (f64, f64))> {
        let imp = self.imp();
        let paintable = imp.picture.paintable()?;
        let (vx, vy, vw, vh) = bounds_rect(&imp.scrolled_window.compute_bounds(&imp.overlay)?);
        let (ix, iy, iw, ih) = bounds_rect(&imp.picture.compute_bounds(&imp.overlay)?);
        let (width, _) = self.image_size(&paintable);
        (iw > 0.0 && ih > 0.0).then(|| {
            (
                iw / width,
                ((vx + vw / 2.0 - ix) / iw, (vy + vh / 2.0 - iy) / ih),
            )
        })
    }

    /// Crop Copy takes over scrolling while it covers the viewer.
    pub fn set_crop_open(&self, open: bool) {
        self.imp().crop_open.set(open);
    }

    pub fn guide_visible(&self) -> bool {
        self.imp().guide_on.get()
    }

    fn setup_guide(&self) {
        let imp = self.imp();
        let w = self.downgrade();
        imp.guide.set_draw(move |snapshot| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            if imp.picture.paintable().is_none() {
                return;
            }
            let (Some(img), Some((guide, view))) = (
                imp.picture.compute_bounds(&imp.overlay),
                viewer.guide_geometry(),
            ) else {
                return;
            };
            use crate::ui::guide_layer as layer;
            // Stay inside the scrolled viewport when the image overflows it.
            snapshot.push_clip(&layer::graphene(view));
            // Dim the part of the image outside the guide, then outline it.
            layer::dim(snapshot, bounds_rect(&img), guide);
            layer::outline(snapshot, crate::palette::GUIDE_EDGE_SHADOW, guide, 4.0);
            layer::outline(snapshot, crate::palette::CROP_EDGE, guide, 2.0);
            // Corner squares to grab, as in Crop Copy (D38).
            layer::handles(snapshot, guide);
            snapshot.pop();
        });

        // D38: corners resize the guide and its outline moves it; presses
        // elsewhere fall through to panning. Capture runs before the
        // scrolled window's pan gesture, so a claimed press never pans.
        let grab = gtk4::GestureDrag::new();
        grab.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let w = self.downgrade();
        grab.connect_drag_begin(move |gesture, x, y| {
            let Some(viewer) = w.upgrade() else { return };
            match viewer.guide_grab_at(x, y) {
                Some(start) => {
                    viewer.imp().guide_drag.set(Some(start));
                    let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
                }
                None => {
                    let _ = gesture.set_state(gtk4::EventSequenceState::Denied);
                }
            }
        });
        let w = self.downgrade();
        grab.connect_drag_update(move |_, dx, dy| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            let (Some((grab, start)), Some((_, view))) =
                (imp.guide_drag.get(), viewer.guide_geometry())
            else {
                return;
            };
            imp.guide_manual.set(Some(match grab {
                GuideGrab::Edge => guide_moved(start, dx, dy, view),
                GuideGrab::Corner(corner) => guide_resized(start, corner, dx, dy, view),
            }));
            imp.guide.queue_draw();
        });
        let w = self.downgrade();
        grab.connect_drag_end(move |_, _, _| {
            if let Some(viewer) = w.upgrade() {
                viewer.imp().guide_drag.set(None);
            }
        });
        imp.overlay.add_controller(grab);

        // Show what a press would grab.
        let hover = gtk4::EventControllerMotion::new();
        let shown = Cell::new(None::<&'static str>);
        let w = self.downgrade();
        hover.connect_motion(move |_, x, y| {
            let Some(viewer) = w.upgrade() else { return };
            let imp = viewer.imp();
            let grabbed = imp.guide_drag.get().or_else(|| viewer.guide_grab_at(x, y));
            let name = grabbed.map(|(grab, _)| {
                use crate::export::Corner;
                match grab {
                    GuideGrab::Corner(Corner::TopLeft | Corner::BottomRight) => "nwse-resize",
                    GuideGrab::Corner(_) => "nesw-resize",
                    GuideGrab::Edge => "move",
                }
            });
            if name != shown.get() {
                shown.set(name);
                imp.overlay.set_cursor_from_name(name);
            }
        });
        imp.overlay.add_controller(hover);

        // The shading follows zoom, pan, resize and image changes: redraw
        // only when the picture or viewport geometry moved, and only while
        // the guide is on.
        let last = Cell::new(None::<[i32; 6]>);
        let w = self.downgrade();
        imp.guide.add_tick_callback(move |guide, _| {
            let Some(viewer) = w.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let imp = viewer.imp();
            if !imp.guide_on.get() {
                last.set(None);
                return glib::ControlFlow::Continue;
            }
            let key = imp.picture.compute_bounds(&imp.overlay).map(|r| {
                [
                    r.x() as i32,
                    r.y() as i32,
                    r.width() as i32,
                    r.height() as i32,
                    imp.scrolled_window.width(),
                    imp.scrolled_window.height(),
                ]
            });
            if key != last.get() {
                last.set(key);
                guide.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.imp().current_path.borrow().clone()
    }

    fn build_ui(&self) {
        let imp = self.imp();
        imp.content_box.set_parent(self);
        self.setup_guide();

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
                if viewer.imp().crop_open.get() {
                    return glib::Propagation::Proceed;
                }
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
        // The view also grows without a remap (fullscreen hides the side
        // panels); an image smaller than the screen must still fill it.
        let imp = self.imp();
        for adjustment in [
            imp.scrolled_window.hadjustment(),
            imp.scrolled_window.vadjustment(),
        ] {
            let w = self.downgrade();
            adjustment.connect_page_size_notify(move |_| {
                if let Some(viewer) = w.upgrade() {
                    viewer.ensure_fit_on_layout();
                }
            });
        }
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

    /// Collapse the pinned or hover-expanded Info card.
    pub fn hide_info(&self) -> bool {
        self.imp().metadata_chip.hide_info()
    }

    /// Pin Info now, or when the current image metadata arrives.
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
        self.forget_close_count();
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
        self.forget_close_count();
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

    pub(crate) fn apply_zoom(&self, factor: f64) {
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

    pub(crate) fn refresh_membership(&self) {
        let imp = self.imp();
        let path = match imp.current_path.borrow().clone() {
            Some(path) => path,
            None => {
                imp.metadata_chip.update_collections(&[]);
                imp.metadata_chip.update_duplicate_count(0);
                return;
            }
        };

        let (index, library_id, roots, disabled) = imp
            .state
            .borrow()
            .as_ref()
            .map(|state| {
                let st = state.borrow();
                (
                    st.library_index.clone(),
                    st.settings.active_library_id.clone(),
                    st.settings.library_roots(),
                    st.disabled_folders.clone(),
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
                    .map(|index| {
                        crate::copies::identical_elsewhere(index, &worker_path, &roots, &disabled)
                    })
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

    /// Drops the close-images count and cancels one under way.
    fn forget_close_count(&self) {
        let imp = self.imp();
        imp.close_gen
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        imp.close_counted.borrow_mut().take();
        imp.metadata_chip.clear_close_count();
    }

    /// Counts the pictures close to the image (item 29, D44) while Info is
    /// open, once per image, off the GTK thread. The vectors load only when
    /// Info first opens, not for every image flipped past.
    pub(crate) fn refresh_close_count(&self) {
        let imp = self.imp();
        if !imp.metadata_chip.is_info_open() {
            return;
        }
        let Some(path) = imp.current_path.borrow().clone() else {
            return;
        };
        if imp.close_counted.borrow().as_ref() == Some(&path) {
            return;
        }
        let Some(searcher) = imp.visual_searcher.borrow().clone() else {
            return;
        };
        let (index, roots, disabled) = imp
            .state
            .borrow()
            .as_ref()
            .map(|state| {
                let st = state.borrow();
                (
                    st.library_index.clone(),
                    st.settings.library_roots(),
                    st.disabled_folders.clone(),
                )
            })
            .unwrap_or_default();
        let Some(index) = index else { return };
        self.forget_close_count();
        *imp.close_counted.borrow_mut() = Some(path.clone());
        let canceled = imp.close_gen.clone();
        let generation = canceled.load(std::sync::atomic::Ordering::Relaxed);
        let worker_path = path.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let superseded = || canceled.load(std::sync::atomic::Ordering::Relaxed) != generation;
            let count = searcher.vectors(&superseded).ok().and_then(|vectors| {
                crate::visual_search::similar::close_count_in_library(
                    &index,
                    &worker_path,
                    &roots,
                    &disabled,
                    &vectors,
                    &superseded,
                )
            });
            let _ = tx.send_blocking(count);
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(count) = rx.recv().await else { return };
            let Some(viewer) = weak.upgrade() else { return };
            let imp = viewer.imp();
            if imp.close_gen.load(std::sync::atomic::Ordering::Relaxed) != generation
                || imp.current_path.borrow().as_ref() != Some(&path)
            {
                return;
            }
            // Not indexed yet: Info tries again the next time it opens.
            if count.is_none() {
                imp.close_counted.borrow_mut().take();
            }
            imp.metadata_chip.update_close_count(&path, count);
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

#[cfg(test)]
mod guide_tests {
    use super::{
        guide_frame, guide_grab, guide_moved, guide_rect, guide_resized, guide_within, GuideGrab,
    };
    use crate::export::Corner;

    fn near(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) {
        assert!(
            (a.0 - b.0).abs() < 1e-9
                && (a.1 - b.1).abs() < 1e-9
                && (a.2 - b.2).abs() < 1e-9
                && (a.3 - b.3).abs() < 1e-9,
            "{a:?} != {b:?}"
        );
    }

    #[test]
    fn exact_16_9_matches_the_image() {
        near(
            guide_rect(10.0, 20.0, 1600.0, 900.0).unwrap(),
            (10.0, 20.0, 1600.0, 900.0),
        );
    }

    #[test]
    fn wider_image_overhangs_at_the_sides() {
        let w = 500.0 * 16.0 / 9.0;
        near(
            guide_rect(0.0, 0.0, 2000.0, 500.0).unwrap(),
            ((2000.0 - w) / 2.0, 0.0, w, 500.0),
        );
    }

    #[test]
    fn taller_landscape_overhangs_above_and_below() {
        near(
            guide_rect(0.0, 0.0, 1600.0, 1200.0).unwrap(),
            (0.0, 150.0, 1600.0, 900.0),
        );
    }

    #[test]
    fn square_and_portrait_get_a_centred_band() {
        near(
            guide_rect(0.0, 0.0, 800.0, 800.0).unwrap(),
            (0.0, 175.0, 800.0, 450.0),
        );
        near(
            guide_rect(5.0, 0.0, 900.0, 1600.0).unwrap(),
            (5.0, 546.875, 900.0, 506.25),
        );
    }

    #[test]
    fn empty_image_has_no_guide() {
        assert!(guide_rect(0.0, 0.0, 0.0, 10.0).is_none());
        assert!(guide_rect(0.0, 0.0, 10.0, -1.0).is_none());
    }

    const VIEW: (f64, f64, f64, f64) = (0.0, 0.0, 1000.0, 600.0);
    const GUIDE: (f64, f64, f64, f64) = (100.0, 100.0, 320.0, 180.0);

    #[test]
    fn corners_resize_the_outline_moves_and_inside_pans() {
        assert_eq!(
            guide_grab(GUIDE, 104.0, 96.0),
            Some(GuideGrab::Corner(Corner::TopLeft))
        );
        assert_eq!(
            guide_grab(GUIDE, 420.0, 280.0),
            Some(GuideGrab::Corner(Corner::BottomRight))
        );
        assert_eq!(guide_grab(GUIDE, 250.0, 102.0), Some(GuideGrab::Edge));
        assert_eq!(guide_grab(GUIDE, 425.0, 190.0), Some(GuideGrab::Edge));
        // Inside, away from the outline, and well outside: left to panning.
        assert_eq!(guide_grab(GUIDE, 250.0, 190.0), None);
        assert_eq!(guide_grab(GUIDE, 600.0, 500.0), None);
    }

    #[test]
    fn moving_keeps_the_guide_on_screen() {
        near(
            guide_moved(GUIDE, 50.0, -20.0, VIEW),
            (150.0, 80.0, 320.0, 180.0),
        );
        near(
            guide_moved(GUIDE, 5000.0, 5000.0, VIEW),
            (680.0, 420.0, 320.0, 180.0),
        );
        near(
            guide_moved(GUIDE, -500.0, 0.0, VIEW),
            (0.0, 100.0, 320.0, 180.0),
        );
    }

    #[test]
    fn resizing_holds_16_9_and_the_opposite_corner() {
        // A drag along the diagonal moves the corner with the pointer.
        let r = guide_resized(GUIDE, Corner::BottomRight, 160.0, 90.0, VIEW);
        near(r, (100.0, 100.0, 480.0, 270.0));
        // From the top left, the bottom right stays put.
        let r = guide_resized(GUIDE, Corner::TopLeft, 160.0, 90.0, VIEW);
        near(r, (260.0, 190.0, 160.0, 90.0));
        // A straight drag still resizes, from either axis.
        for (dx, dy) in [(160.0, 0.0), (0.0, 90.0)] {
            let r = guide_resized(GUIDE, Corner::BottomRight, dx, dy, VIEW);
            assert!(r.2 > 320.0 && (r.2 * 9.0 - r.3 * 16.0).abs() < 1e-9);
            assert_eq!((r.0, r.1), (100.0, 100.0));
        }
        // Never past the screen, nor below the smallest size.
        let r = guide_resized(GUIDE, Corner::BottomRight, 5000.0, 0.0, VIEW);
        assert!(r.0 + r.2 <= 1000.0 + 1e-9 && r.1 + r.3 <= 600.0 + 1e-9);
        assert!((r.2 * 9.0 - r.3 * 16.0).abs() < 1e-9);
        let r = guide_resized(GUIDE, Corner::BottomRight, -5000.0, 0.0, VIEW);
        near(r, (100.0, 100.0, 96.0, 54.0));
    }

    #[test]
    fn resizing_never_jumps() {
        // Growing one way while shrinking the other changes the size a
        // little for a little movement, never by a leap.
        let mut last = guide_resized(GUIDE, Corner::BottomRight, 0.0, 0.0, VIEW).2;
        for step in 1..=200 {
            let t = f64::from(step);
            let width = guide_resized(GUIDE, Corner::BottomRight, t, -t * 0.9, VIEW).2;
            assert!((width - last).abs() < 1.0, "{last} -> {width}");
            last = width;
        }
    }

    #[test]
    fn a_smaller_window_shrinks_the_guide_about_its_centre() {
        near(guide_within(GUIDE, VIEW), GUIDE);
        let big = (0.0, 0.0, 1600.0, 900.0);
        // 1000 wide is all the room there is; centred, then shifted on screen.
        near(guide_within(big, VIEW), (0.0, 37.5, 1000.0, 562.5));
    }

    #[test]
    fn the_frame_is_the_guide_s_share_of_the_image() {
        let image = (0.0, 0.0, 1000.0, 500.0);
        near(
            guide_frame((250.0, 125.0, 500.0, 250.0), image).unwrap(),
            (0.25, 0.25, 0.5, 0.5),
        );
        // Overhanging the image: only the overlap.
        near(
            guide_frame((750.0, 0.0, 500.0, 500.0), image).unwrap(),
            (0.75, 0.0, 0.25, 1.0),
        );
        assert!(guide_frame((2000.0, 0.0, 10.0, 10.0), image).is_none());
    }
}
