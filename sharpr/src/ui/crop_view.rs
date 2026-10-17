//! Where Crop Copy puts the image on its canvas: a zoom and the image point
//! at the canvas centre. Pure geometry, so zoom and pan are tested without
//! GTK. The selection is kept in image pixels, so it stays attached to the
//! image whatever the view does.

/// The farthest zoom, as in the viewer: 20 times the larger of the fit and
/// 1:1 scales.
const ZOOM_REACH: f64 = 20.0;

/// How the image sits on a canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropView {
    /// Times the fit scale, at least 1 (1 shows the whole image).
    pub zoom: f64,
    /// The image pixel at the canvas centre.
    pub centre: (f64, f64),
}

/// Screen pixels per image pixel when the whole image just fits.
pub fn fit_scale(dims: (u32, u32), (width, height): (f64, f64)) -> f64 {
    if width <= 0.0 || height <= 0.0 || dims.0 == 0 || dims.1 == 0 {
        return 1.0;
    }
    (width / f64::from(dims.0)).min(height / f64::from(dims.1))
}

/// The largest zoom for an image on a canvas of `size`.
pub fn max_zoom(dims: (u32, u32), size: (f64, f64)) -> f64 {
    (1.0 / fit_scale(dims, size).max(f64::EPSILON)).max(1.0) * ZOOM_REACH
}

/// Whether the texture, `texture_width` pixels across for an image `image_width`
/// across, has too few pixels for the view's `scale` on a display with
/// `device_scale`. A texture as wide as the image never does.
pub fn needs_full_resolution(
    texture_width: u32,
    image_width: u32,
    scale: f64,
    device_scale: f64,
) -> bool {
    if texture_width >= image_width || image_width == 0 {
        return false;
    }
    let density = f64::from(texture_width) / f64::from(image_width);
    scale * device_scale > density * 1.02
}

impl CropView {
    /// The whole image, centred.
    pub fn fit(dims: (u32, u32)) -> Self {
        Self {
            zoom: 1.0,
            centre: (f64::from(dims.0) / 2.0, f64::from(dims.1) / 2.0),
        }
    }

    /// A view showing the image at `scale` screen pixels per image pixel,
    /// its centre at fractions of the image (`fraction.0` of the width,
    /// `fraction.1` of the height), such as the viewer's, kept valid for the
    /// canvas: never smaller than the whole image.
    pub fn from_scale(
        scale: f64,
        fraction: (f64, f64),
        dims: (u32, u32),
        size: (f64, f64),
    ) -> Self {
        Self {
            zoom: scale / fit_scale(dims, size).max(f64::EPSILON),
            centre: (
                fraction.0 * f64::from(dims.0),
                fraction.1 * f64::from(dims.1),
            ),
        }
        .clamped(dims, size)
    }

    /// Screen pixels per image pixel.
    pub fn scale(self, dims: (u32, u32), size: (f64, f64)) -> f64 {
        fit_scale(dims, size) * self.zoom
    }

    /// Where the image's top-left corner lands on the canvas, and the scale.
    pub fn placement(self, dims: (u32, u32), size: (f64, f64)) -> (f64, f64, f64) {
        let scale = self.scale(dims, size);
        (
            size.0 / 2.0 - self.centre.0 * scale,
            size.1 / 2.0 - self.centre.1 * scale,
            scale,
        )
    }

    /// The view with its zoom within reach and the image covering the canvas
    /// on every axis it is larger than it (centred on the others).
    pub fn clamped(self, dims: (u32, u32), size: (f64, f64)) -> Self {
        let zoom = self.zoom.clamp(1.0, max_zoom(dims, size));
        let scale = fit_scale(dims, size) * zoom;
        let axis = |centre: f64, image: u32, canvas: f64| {
            let image = f64::from(image);
            let half = canvas / 2.0 / scale;
            if half * 2.0 >= image {
                image / 2.0
            } else {
                centre.clamp(half, image - half)
            }
        };
        Self {
            zoom,
            centre: (
                axis(self.centre.0, dims.0, size.0),
                axis(self.centre.1, dims.1, size.1),
            ),
        }
    }

    /// The view after zooming by `factor` with the image pixel under the
    /// canvas point `at` staying under it.
    pub fn zoomed_at(
        self,
        factor: f64,
        at: (f64, f64),
        dims: (u32, u32),
        size: (f64, f64),
    ) -> Self {
        let (ox, oy, scale) = self.placement(dims, size);
        let under = ((at.0 - ox) / scale, (at.1 - oy) / scale);
        let zoom = (self.zoom * factor).clamp(1.0, max_zoom(dims, size));
        let new_scale = fit_scale(dims, size) * zoom;
        Self {
            zoom,
            centre: (
                under.0 + (size.0 / 2.0 - at.0) / new_scale,
                under.1 + (size.1 / 2.0 - at.1) / new_scale,
            ),
        }
        .clamped(dims, size)
    }

    /// The view after dragging the image by (`dx`, `dy`) canvas pixels.
    pub fn panned(self, dx: f64, dy: f64, dims: (u32, u32), size: (f64, f64)) -> Self {
        let scale = self.scale(dims, size);
        Self {
            zoom: self.zoom,
            centre: (self.centre.0 - dx / scale, self.centre.1 - dy / scale),
        }
        .clamped(dims, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIMS: (u32, u32) = (4000, 2000);
    const SIZE: (f64, f64) = (1000.0, 800.0);

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn fit_shows_the_whole_image_centred() {
        let (ox, oy, scale) = CropView::fit(DIMS).placement(DIMS, SIZE);
        close(scale, 0.25);
        close(ox, 0.0);
        close(oy, 150.0);
    }

    #[test]
    fn a_small_image_fills_the_canvas_at_fit() {
        let (ox, oy, scale) = CropView::fit((100, 100)).placement((100, 100), SIZE);
        close(scale, 8.0);
        close(ox, 100.0);
        close(oy, 0.0);
    }

    #[test]
    fn the_image_pixel_under_the_pointer_stays_put() {
        let at = (700.0, 300.0);
        let before = CropView::fit(DIMS);
        let (ox, oy, scale) = before.placement(DIMS, SIZE);
        let under = ((at.0 - ox) / scale, (at.1 - oy) / scale);
        // Both axes overflow the canvas from 2x on; below that the short
        // axis stays centred, as in the viewer.
        for factor in [2.0, 7.0] {
            let after = before.zoomed_at(factor, at, DIMS, SIZE);
            let (ox, oy, scale) = after.placement(DIMS, SIZE);
            close((at.0 - ox) / scale, under.0);
            close((at.1 - oy) / scale, under.1);
        }
    }

    #[test]
    fn zooming_out_to_fit_recentres_the_image() {
        let zoomed = CropView::fit(DIMS).zoomed_at(4.0, (900.0, 700.0), DIMS, SIZE);
        assert!(zoomed.zoom > 1.0);
        let fit = zoomed.zoomed_at(0.01, (900.0, 700.0), DIMS, SIZE);
        assert_eq!(fit, CropView::fit(DIMS));
    }

    #[test]
    fn zoom_stays_within_reach() {
        let most = max_zoom(DIMS, SIZE);
        // 1 / 0.25 = 4 times 1:1, times 20.
        close(most, 80.0);
        let view = CropView::fit(DIMS).zoomed_at(1e9, (500.0, 400.0), DIMS, SIZE);
        close(view.zoom, most);
        // A small image already above 1:1 at fit reaches 20 times fit.
        close(max_zoom((100, 100), SIZE), 20.0);
    }

    #[test]
    fn panning_moves_the_image_with_the_drag_and_stops_at_its_edges() {
        let zoomed = CropView::fit(DIMS).zoomed_at(4.0, (500.0, 400.0), DIMS, SIZE);
        let (ox, oy, scale) = zoomed.placement(DIMS, SIZE);
        let moved = zoomed.panned(30.0, -20.0, DIMS, SIZE);
        let (nx, ny, nscale) = moved.placement(DIMS, SIZE);
        close(nscale, scale);
        close(nx, ox + 30.0);
        close(ny, oy - 20.0);
        // Far enough, the image's corner meets the canvas corner.
        let end = zoomed.panned(1e9, 1e9, DIMS, SIZE);
        let (ex, ey, _) = end.placement(DIMS, SIZE);
        close(ex, 0.0);
        close(ey, 0.0);
        let end = zoomed.panned(-1e9, -1e9, DIMS, SIZE);
        let (ex, ey, scale) = end.placement(DIMS, SIZE);
        close(ex + f64::from(DIMS.0) * scale, SIZE.0);
        close(ey + f64::from(DIMS.1) * scale, SIZE.1);
    }

    #[test]
    fn panning_at_fit_does_not_move_the_image() {
        let fit = CropView::fit(DIMS);
        assert_eq!(fit.panned(200.0, 100.0, DIMS, SIZE), fit);
    }

    #[test]
    fn the_viewer_s_view_carries_over() {
        // Shown at 1 screen pixel per image pixel (4 times the fit scale),
        // looking at the image's top right quarter.
        let view = CropView::from_scale(1.0, (0.75, 0.25), DIMS, SIZE);
        close(view.zoom, 4.0);
        close(view.centre.0, 3000.0);
        close(view.centre.1, 500.0);
        close(view.scale(DIMS, SIZE), 1.0);
        // The scale holds on a wider canvas, where the fit scale differs.
        let wide = (1600.0, 800.0);
        let view = CropView::from_scale(1.0, (0.5, 0.5), DIMS, wide);
        close(view.scale(DIMS, wide), 1.0);
        // The whole image or less is the whole image, centred, whatever
        // was asked.
        let fit = CropView::from_scale(0.1, (0.1, 0.9), DIMS, SIZE);
        assert_eq!(fit, CropView::fit(DIMS));
    }

    #[test]
    fn a_resized_canvas_keeps_the_centre_and_refits() {
        let view = CropView::fit(DIMS).zoomed_at(4.0, (500.0, 400.0), DIMS, SIZE);
        let bigger = (2000.0, 1000.0);
        let after = view.clamped(DIMS, bigger);
        assert_eq!(after.zoom, view.zoom);
        let (_, _, scale) = after.placement(DIMS, bigger);
        close(scale, 0.5 * 4.0);
        assert_eq!(
            CropView::fit(DIMS).clamped(DIMS, bigger),
            CropView::fit(DIMS)
        );
    }

    #[test]
    fn only_a_texture_short_of_pixels_needs_the_full_image() {
        // A 2560 preview of a 10000 pixel image: enough at 0.25, not at 1.
        assert!(!needs_full_resolution(2560, 10000, 0.25, 1.0));
        assert!(needs_full_resolution(2560, 10000, 1.0, 1.0));
        // A high-density display needs the pixels sooner.
        assert!(needs_full_resolution(2560, 10000, 0.25, 2.0));
        // The full image never needs another.
        assert!(!needs_full_resolution(10000, 10000, 20.0, 2.0));
    }
}
