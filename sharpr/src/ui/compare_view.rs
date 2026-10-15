//! The two-image canvas of Compare (`ROADMAP.md` Phase 8, decision D8):
//! side by side, a draggable split, or A/B flip, with zoom and pan that can
//! be locked so both images show the same region.

use std::cell::{Cell, RefCell};

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{glib, graphene, gsk};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    SideBySide,
    Split,
    Flip,
}

/// Zoom and pan of one image. `zoom` is screen pixels per image pixel of
/// the reference image (`None` fits the pane); `center` is the point of the
/// picture, from 0 to 1 in each direction, at the middle of the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: Option<f64>,
    pub center: (f64, f64),
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: None,
            center: (0.5, 0.5),
        }
    }
}

/// Where an image of `size` pixels is drawn in a pane of `pane` size:
/// (x, y, width, height). `reference_width` makes zoom mean the same
/// region for images of different resolutions: a 1080p and a 4K version
/// of one picture show the same part of it at the same zoom.
pub fn placement(
    pane: (f64, f64),
    size: (f64, f64),
    view: View,
    reference_width: f64,
) -> (f64, f64, f64, f64) {
    let fit = (pane.0 / size.0).min(pane.1 / size.1);
    let scale = view
        .zoom
        .map(|zoom| zoom * reference_width / size.0)
        .unwrap_or(fit);
    let (w, h) = (size.0 * scale, size.1 * scale);
    let place = |pane: f64, extent: f64, center: f64| {
        if extent <= pane {
            (pane - extent) / 2.0
        } else {
            (pane / 2.0 - center * extent).clamp(pane - extent, 0.0)
        }
    };
    (
        place(pane.0, w, view.center.0),
        place(pane.1, h, view.center.1),
        w,
        h,
    )
}

/// `view` zoomed by `factor` around the pane point `at`, keeping the
/// picture point under it in place.
pub fn zoomed(
    view: View,
    factor: f64,
    at: (f64, f64),
    pane: (f64, f64),
    size: (f64, f64),
    reference_width: f64,
) -> View {
    let (x, y, w, h) = placement(pane, size, view, reference_width);
    let current = w / size.0 * size.0 / reference_width;
    let zoom = (current * factor).clamp(0.02, 32.0);
    // The picture point under the pointer, which must stay under it.
    let point = ((at.0 - x) / w, (at.1 - y) / h);
    let (new_w, new_h) = (w * factor, h * factor);
    let center = (
        point.0 + (pane.0 / 2.0 - at.0) / new_w,
        point.1 + (pane.1 / 2.0 - at.1) / new_h,
    );
    View {
        zoom: Some(zoom),
        center: (center.0.clamp(0.0, 1.0), center.1.clamp(0.0, 1.0)),
    }
}

/// `view` panned by a drag of (`dx`, `dy`) screen pixels.
pub fn panned(view: View, dx: f64, dy: f64, drawn: (f64, f64)) -> View {
    View {
        center: (
            (view.center.0 - dx / drawn.0).clamp(0.0, 1.0),
            (view.center.1 - dy / drawn.1).clamp(0.0, 1.0),
        ),
        ..view
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct CompareView {
        pub textures: RefCell<[Option<gdk4::Texture>; 2]>,
        pub mode: Cell<Mode>,
        pub locked: Cell<bool>,
        /// One view when locked; one per image otherwise.
        pub views: Cell<[View; 2]>,
        /// Where the split divides, from 0 to 1 across the width.
        pub split: Cell<f64>,
        /// In flip mode, whether the right image is the one shown.
        pub flipped: Cell<bool>,
        pub drag: Cell<Option<DragStart>>,
    }

    #[derive(Clone, Copy)]
    pub enum DragStart {
        Pan([View; 2], usize),
        Divider(f64),
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompareView {
        const NAME: &'static str = "SharprCompareView";
        type Type = super::CompareView;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for CompareView {
        fn constructed(&self) {
            self.parent_constructed();
            self.split.set(0.5);
            self.locked.set(true);
            self.obj().set_hexpand(true);
            self.obj().set_vexpand(true);
            self.obj().set_focusable(true);
            super::install_controllers(&self.obj());
        }
    }

    impl WidgetImpl for CompareView {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            super::draw(&self.obj(), snapshot);
        }
    }
}

glib::wrapper! {
    pub struct CompareView(ObjectSubclass<imp::CompareView>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Default for CompareView {
    fn default() -> Self {
        glib::Object::new()
    }
}

fn size_of(texture: &gdk4::Texture) -> (f64, f64) {
    (f64::from(texture.width()), f64::from(texture.height()))
}

impl CompareView {
    pub fn set_texture(&self, side: usize, texture: Option<gdk4::Texture>) {
        self.imp().textures.borrow_mut()[side] = texture;
        self.queue_draw();
    }

    pub fn set_mode(&self, mode: Mode) {
        self.imp().mode.set(mode);
        self.queue_draw();
    }

    pub fn mode(&self) -> Mode {
        self.imp().mode.get()
    }

    /// Locking makes the right image follow the left one's view.
    pub fn set_locked(&self, locked: bool) {
        let imp = self.imp();
        imp.locked.set(locked);
        if locked {
            let views = imp.views.get();
            imp.views.set([views[0], views[0]]);
        }
        self.queue_draw();
    }

    pub fn locked(&self) -> bool {
        self.imp().locked.get()
    }

    /// Shows the other image in flip mode.
    pub fn flip(&self) {
        let imp = self.imp();
        imp.flipped.set(!imp.flipped.get());
        self.queue_draw();
    }

    pub fn flipped(&self) -> bool {
        self.imp().flipped.get()
    }

    /// `None` fits; otherwise screen pixels per pixel of the left image.
    pub fn set_zoom(&self, zoom: Option<f64>) {
        let imp = self.imp();
        let views = imp.views.get().map(|view| View { zoom, ..view });
        imp.views.set(views);
        self.queue_draw();
    }

    pub fn views(&self) -> [View; 2] {
        self.imp().views.get()
    }

    pub fn set_split(&self, split: f64) {
        self.imp().split.set(split.clamp(0.0, 1.0));
        self.queue_draw();
    }

    pub fn split(&self) -> f64 {
        self.imp().split.get()
    }

    /// The panes: side by side has two halves; split and flip draw both
    /// images in the same pane.
    fn panes(&self) -> [(f64, f64, f64, f64); 2] {
        let (w, h) = (f64::from(self.width()), f64::from(self.height()));
        match self.mode() {
            Mode::SideBySide => [(0.0, 0.0, w / 2.0, h), (w / 2.0, 0.0, w / 2.0, h)],
            Mode::Split | Mode::Flip => [(0.0, 0.0, w, h); 2],
        }
    }

    /// The width that defines zoom for `side`: the left image's when the
    /// views are locked or the images share a pane, so both show the same
    /// region; otherwise the image's own.
    fn reference_width(&self, side: usize) -> f64 {
        let textures = self.imp().textures.borrow();
        let own = textures[side].as_ref().map(|t| size_of(t).0).unwrap_or(1.0);
        if self.locked() || self.mode() != Mode::SideBySide {
            textures[0].as_ref().map(|t| size_of(t).0).unwrap_or(own)
        } else {
            own
        }
    }

    fn view_for(&self, side: usize) -> View {
        let views = self.imp().views.get();
        if self.locked() || self.mode() != Mode::SideBySide {
            views[0]
        } else {
            views[side]
        }
    }

    /// Which image a pane point belongs to.
    fn side_at(&self, x: f64) -> usize {
        match self.mode() {
            Mode::SideBySide => usize::from(x >= f64::from(self.width()) / 2.0),
            Mode::Split => usize::from(x >= self.split() * f64::from(self.width())),
            Mode::Flip => usize::from(self.flipped()),
        }
    }

    fn set_view_for(&self, side: usize, view: View) {
        let imp = self.imp();
        let mut views = imp.views.get();
        if self.locked() || self.mode() != Mode::SideBySide {
            views = [view, view];
        } else {
            views[side] = view;
        }
        imp.views.set(views);
        self.queue_draw();
    }

    fn zoom_at(&self, factor: f64, x: f64, y: f64) {
        let side = self.side_at(x);
        let Some(size) = self.imp().textures.borrow()[side].as_ref().map(size_of) else {
            return;
        };
        let pane = self.panes()[side];
        let view = zoomed(
            self.view_for(side),
            factor,
            (x - pane.0, y - pane.1),
            (pane.2, pane.3),
            size,
            self.reference_width(side),
        );
        self.set_view_for(side, view);
    }
}

fn draw(view: &CompareView, snapshot: &gtk4::Snapshot) {
    let imp = view.imp();
    let textures = imp.textures.borrow();
    let panes = view.panes();
    let (w, h) = (f64::from(view.width()), f64::from(view.height()));
    let draw_side = |side: usize, clip: (f64, f64, f64, f64)| {
        let Some(texture) = textures[side].as_ref() else {
            return;
        };
        let pane = panes[side];
        let (x, y, dw, dh) = placement(
            (pane.2, pane.3),
            size_of(texture),
            view.view_for(side),
            view.reference_width(side),
        );
        let scale = dw / size_of(texture).0;
        // Inspecting pixels: no smoothing past 2×.
        let filter = if scale >= 2.0 {
            gsk::ScalingFilter::Nearest
        } else {
            gsk::ScalingFilter::Trilinear
        };
        snapshot.push_clip(&graphene::Rect::new(
            clip.0 as f32,
            clip.1 as f32,
            clip.2 as f32,
            clip.3 as f32,
        ));
        snapshot.append_scaled_texture(
            texture,
            filter,
            &graphene::Rect::new(
                (pane.0 + x) as f32,
                (pane.1 + y) as f32,
                dw as f32,
                dh as f32,
            ),
        );
        snapshot.pop();
    };
    match view.mode() {
        Mode::SideBySide => {
            draw_side(0, panes[0]);
            draw_side(1, panes[1]);
            snapshot.append_color(
                &super::look::gdk_color(crate::palette::SCRIM),
                &graphene::Rect::new((w / 2.0 - 1.0) as f32, 0.0, 2.0, h as f32),
            );
        }
        Mode::Split => {
            let at = view.split() * w;
            draw_side(0, (0.0, 0.0, at, h));
            draw_side(1, (at, 0.0, w - at, h));
            let white = super::look::gdk_color(crate::palette::DIVIDER);
            snapshot.append_color(
                &white,
                &graphene::Rect::new((at - 1.0) as f32, 0.0, 2.0, h as f32),
            );
            snapshot.append_color(
                &white,
                &graphene::Rect::new((at - 6.0) as f32, (h / 2.0 - 20.0) as f32, 12.0, 40.0),
            );
        }
        Mode::Flip => draw_side(usize::from(view.flipped()), (0.0, 0.0, w, h)),
    }
}

fn install_controllers(view: &CompareView) {
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let pointer = std::rc::Rc::new(Cell::new((0.0, 0.0)));
    {
        let pointer = pointer.clone();
        let motion = gtk4::EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| pointer.set((x, y)));
        view.add_controller(motion);
    }
    {
        let weak = view.downgrade();
        scroll.connect_scroll(move |_, _, dy| {
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let (x, y) = pointer.get();
            view.zoom_at(if dy < 0.0 { 1.25 } else { 0.8 }, x, y);
            glib::Propagation::Stop
        });
    }
    view.add_controller(scroll);

    let drag = gtk4::GestureDrag::new();
    {
        let weak = view.downgrade();
        drag.connect_drag_begin(move |_, x, _| {
            let Some(view) = weak.upgrade() else { return };
            view.grab_focus();
            let divider = view.split() * f64::from(view.width());
            let start = if view.mode() == Mode::Split && (x - divider).abs() <= 16.0 {
                imp::DragStart::Divider(view.split())
            } else {
                imp::DragStart::Pan(view.imp().views.get(), view.side_at(x))
            };
            view.imp().drag.set(Some(start));
        });
    }
    {
        let weak = view.downgrade();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(view) = weak.upgrade() else { return };
            match view.imp().drag.get() {
                Some(imp::DragStart::Divider(start)) => {
                    view.set_split(start + dx / f64::from(view.width().max(1)));
                }
                Some(imp::DragStart::Pan(views, side)) => {
                    let Some(size) = view.imp().textures.borrow()[side].as_ref().map(size_of)
                    else {
                        return;
                    };
                    let pane = view.panes()[side];
                    let start = if view.locked() || view.mode() != Mode::SideBySide {
                        views[0]
                    } else {
                        views[side]
                    };
                    let (_, _, dw, dh) =
                        placement((pane.2, pane.3), size, start, view.reference_width(side));
                    view.set_view_for(side, panned(start, dx, dy, (dw, dh)));
                }
                None => {}
            }
        });
    }
    {
        let weak = view.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(view) = weak.upgrade() {
                view.imp().drag.set(None);
            }
        });
    }
    view.add_controller(drag);

    // Double-click: fit, or 100% at the pointer.
    let click = gtk4::GestureClick::new();
    {
        let weak = view.downgrade();
        click.connect_pressed(move |_, presses, x, y| {
            if presses != 2 {
                return;
            }
            let Some(view) = weak.upgrade() else { return };
            let side = view.side_at(x);
            if view.view_for(side).zoom.is_some() {
                view.set_zoom(None);
                view.imp().views.set([View::default(); 2]);
                view.queue_draw();
            } else {
                let Some(size) = view.imp().textures.borrow()[side].as_ref().map(size_of) else {
                    return;
                };
                let pane = view.panes()[side];
                let reference = view.reference_width(side);
                let (_, _, dw, _) = placement((pane.2, pane.3), size, View::default(), reference);
                let fit_zoom = dw / reference;
                view.zoom_at(1.0 / fit_zoom, x, y);
            }
        });
    }
    view.add_controller(click);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_centres_the_image_in_its_pane() {
        let (x, y, w, h) = placement((800.0, 600.0), (1600.0, 900.0), View::default(), 1600.0);
        assert_eq!((x, w, h), (0.0, 800.0, 450.0));
        assert_eq!(y, 75.0);
    }

    #[test]
    fn locked_zoom_shows_the_same_region_of_versions_at_other_sizes() {
        let view = View {
            zoom: Some(1.0),
            center: (0.25, 0.5),
        };
        // The 4K original at 100%, and its 1080p version measured in the
        // original's pixels: both are drawn at the same size, so the same
        // part of the picture fills the pane.
        let original = placement((1000.0, 800.0), (3840.0, 2160.0), view, 3840.0);
        let small = placement((1000.0, 800.0), (1920.0, 1080.0), view, 3840.0);
        assert_eq!(original, small);
        assert_eq!(original.2, 3840.0, "100% of the original");
        // The point a quarter of the way across sits in the middle.
        assert!((original.0 + 0.25 * original.2 - 500.0).abs() < 1e-9);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let pane = (1000.0, 800.0);
        let size = (4000.0, 2000.0);
        let start = View::default();
        let at = (700.0, 300.0);
        let before = placement(pane, size, start, size.0);
        let point = ((at.0 - before.0) / before.2, (at.1 - before.1) / before.3);
        let after = zoomed(start, 2.0, at, pane, size, size.0);
        let (x, y, w, h) = placement(pane, size, after, size.0);
        assert!((x + point.0 * w - at.0).abs() < 1.0, "{x} {w}");
        assert!((y + point.1 * h - at.1).abs() < 1.0, "{y} {h}");
        assert!(
            (after.zoom.unwrap() - 0.5).abs() < 1e-9,
            "fit was 0.25; doubled"
        );
    }

    #[test]
    fn panning_moves_the_centre_and_stays_on_the_picture() {
        let view = View {
            zoom: Some(1.0),
            center: (0.5, 0.5),
        };
        let moved = panned(view, -100.0, 0.0, (1000.0, 500.0));
        assert!((moved.center.0 - 0.6).abs() < 1e-9);
        assert_eq!(panned(view, -99_999.0, 0.0, (1000.0, 500.0)).center.0, 1.0);
    }
}
