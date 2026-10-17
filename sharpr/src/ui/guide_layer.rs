//! The layer the viewer's 16:9 guide (D36, D38) and Crop Copy's selection
//! draw on. It draws with GTK snapshot nodes, which the GPU composites, so
//! moving or resizing a frame redraws a few rectangles instead of repainting
//! a viewer-sized Cairo surface on the processor for every pointer movement.

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::RefCell;

type Draw = Box<dyn Fn(&gtk4::Snapshot)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct GuideLayer {
        pub draw: RefCell<Option<Draw>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GuideLayer {
        const NAME: &'static str = "SharprGuideLayer";
        type Type = super::GuideLayer;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for GuideLayer {}

    impl WidgetImpl for GuideLayer {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            if let Some(draw) = self.draw.borrow().as_ref() {
                draw(snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct GuideLayer(ObjectSubclass<imp::GuideLayer>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Default for GuideLayer {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl GuideLayer {
    pub fn set_draw(&self, draw: impl Fn(&gtk4::Snapshot) + 'static) {
        *self.imp().draw.borrow_mut() = Some(Box::new(draw));
        self.queue_draw();
    }
}

/// A rectangle: x, y, width, height.
pub type Rect = (f64, f64, f64, f64);

pub fn graphene((x, y, w, h): Rect) -> gtk4::graphene::Rect {
    gtk4::graphene::Rect::new(x as f32, y as f32, w as f32, h as f32)
}

pub fn fill(snapshot: &gtk4::Snapshot, ink: crate::palette::Ink, rect: Rect) {
    snapshot.append_color(&super::look::gdk_color(ink), &graphene(rect));
}

/// An outline `width` thick, centred on `rect`'s edges.
pub fn outline(
    snapshot: &gtk4::Snapshot,
    ink: crate::palette::Ink,
    (x, y, w, h): Rect,
    width: f64,
) {
    let half = width / 2.0;
    let rounded = gtk4::gsk::RoundedRect::from_rect(
        graphene((x - half, y - half, w + width, h + width)),
        0.0,
    );
    let color = super::look::gdk_color(ink);
    snapshot.append_border(&rounded, &[width as f32; 4], &[color; 4]);
}

/// Dims the part of `image` outside `frame`.
pub fn dim(snapshot: &gtk4::Snapshot, image: Rect, frame: Rect) {
    for band in dim_bands(image, frame) {
        fill(snapshot, crate::palette::SCRIM, band);
    }
}

/// The corner squares to grab a frame by.
pub fn handles(snapshot: &gtk4::Snapshot, (x, y, w, h): Rect) {
    for (cx, cy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
        fill(
            snapshot,
            crate::palette::CROP_EDGE,
            (cx - 5.0, cy - 5.0, 10.0, 10.0),
        );
    }
}

/// The parts of `image` outside `frame`, as up to four bands (above, below,
/// left, right). Edges snap to whole pixels: bands meeting part-way through a
/// pixel would each half-shade it and leave a faint line where they join.
pub fn dim_bands((ix, iy, iw, ih): Rect, (fx, fy, fw, fh): Rect) -> Vec<Rect> {
    let (ix, iy) = (ix.round(), iy.round());
    let (right, bottom) = ((ix + iw).round(), (iy + ih).round());
    let iw = right - ix;
    // The frame clipped to the image; the bands surround it.
    let top = fy.round().clamp(iy, bottom);
    let low = (fy + fh).round().clamp(iy, bottom);
    let left = fx.round().clamp(ix, right);
    let east = (fx + fw).round().clamp(ix, right);
    [
        (ix, iy, iw, top - iy),
        (ix, low, iw, bottom - low),
        (ix, top, left - ix, low - top),
        (east, top, right - east, low - top),
    ]
    .into_iter()
    .filter(|&(_, _, w, h)| w > 0.0 && h > 0.0)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::dim_bands;

    #[test]
    fn dimming_covers_the_image_outside_the_guide() {
        let image = (0.0, 0.0, 1000.0, 1000.0);
        let bands = dim_bands(image, (100.0, 200.0, 800.0, 450.0));
        let area: f64 = bands.iter().map(|b| b.2 * b.3).sum();
        assert!((area - (1000.0 * 1000.0 - 800.0 * 450.0)).abs() < 1e-6);
        // A guide wider than the image leaves only the bands above and below.
        let bands = dim_bands((0.0, 0.0, 400.0, 900.0), (-100.0, 300.0, 600.0, 337.5));
        assert_eq!(bands.len(), 2);
        assert!(dim_bands(image, image).is_empty());
    }
}
