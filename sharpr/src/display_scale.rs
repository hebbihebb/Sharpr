//! Screen-sized copies of very large decoded images for the viewer.
//!
//! Showing a full-resolution texture makes GTK upload every pixel to the GPU
//! on the main thread: on October 13 a very large image stalled the main loop
//! for 434 ms. The viewer instead shows a copy that fits the screen, made on a
//! worker, and loads full resolution only when the user zooms in past it.

/// The size of the copy to show for a `width`×`height` image on a screen of
/// `screen` device pixels, or `None` when the image is small enough to show
/// as it is. The copy fits inside the screen, so the fitted view is as sharp
/// as the original would be.
pub fn display_size(width: u32, height: u32, screen: (u32, u32)) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let factor =
        (screen.0.max(1) as f64 / width as f64).min(screen.1.max(1) as f64 / height as f64);
    // Worth a copy only when it saves at least about a quarter of the pixels.
    if factor >= 0.87 {
        return None;
    }
    Some((
        ((width as f64 * factor).ceil() as u32).max(1),
        ((height as f64 * factor).ceil() as u32).max(1),
    ))
}

/// Bytes per pixel of the memory formats with 8-bit channels. Copies keep
/// the source's format, so only these are reduced; 16-bit and float images
/// are shown as they are.
pub fn bytes_per_pixel(format: glycin::MemoryFormat) -> Option<usize> {
    use glycin::MemoryFormat as F;
    match format {
        F::B8g8r8a8Premultiplied
        | F::A8r8g8b8Premultiplied
        | F::R8g8b8a8Premultiplied
        | F::B8g8r8a8
        | F::A8r8g8b8
        | F::R8g8b8a8
        | F::A8b8g8r8 => Some(4),
        F::R8g8b8 | F::B8g8r8 => Some(3),
        F::G8a8Premultiplied | F::G8a8 => Some(2),
        F::G8 => Some(1),
        _ => None,
    }
}

/// Shrinks an image with 8-bit channels to exactly `target`, averaging each
/// channel independently, so any channel order is kept. `stride` is the
/// source's row length in bytes. Returns tightly packed rows
/// (`target.0 * bpp` bytes each).
pub fn downscale(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    bpp: usize,
    target: (u32, u32),
) -> Vec<u8> {
    let (tw, th) = (target.0.max(1) as usize, target.1.max(1) as usize);
    // A whole-block average first brings huge images close to the target
    // cheaply; an area-weighted average then gives the exact size.
    let k = (width as usize / tw).min(height as usize / th).max(1);
    let (bw, bh) = (width as usize / k, height as usize / k);
    if k == 1 {
        return area_resize(src, stride, bpp, (bw, bh), (tw, th));
    }
    let boxed = box_average(src, stride, bpp, k, bw, bh);
    if (bw, bh) == (tw, th) {
        return boxed;
    }
    area_resize(&boxed, bw * bpp, bpp, (bw, bh), (tw, th))
}

fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8)
}

/// Fills `out` (rows of `row_len` bytes) in parallel bands; `fill(y, row)`
/// writes output row `y`.
fn fill_rows(out: &mut [u8], row_len: usize, fill: impl Fn(usize, &mut [u8]) + Sync) {
    let rows = out.len() / row_len.max(1);
    let per_band = rows.div_ceil(worker_count()).max(1);
    std::thread::scope(|scope| {
        for (band, chunk) in out.chunks_mut(row_len * per_band).enumerate() {
            let fill = &fill;
            scope.spawn(move || {
                for (i, row) in chunk.chunks_mut(row_len).enumerate() {
                    fill(band * per_band + i, row);
                }
            });
        }
    });
}

/// Averages `k`×`k` blocks into a packed `bw`×`bh` image.
fn box_average(src: &[u8], stride: usize, bpp: usize, k: usize, bw: usize, bh: usize) -> Vec<u8> {
    let row_len = bw * bpp;
    let mut out = vec![0u8; row_len * bh];
    let area = (k * k) as u32;
    fill_rows(&mut out, row_len, |y, out_row| {
        let mut sums = vec![0u32; row_len];
        for sy in y * k..y * k + k {
            let row = &src[sy * stride..sy * stride + bw * k * bpp];
            for (sum, block) in sums.chunks_exact_mut(bpp).zip(row.chunks_exact(k * bpp)) {
                for pixel in block.chunks_exact(bpp) {
                    for (s, v) in sum.iter_mut().zip(pixel) {
                        *s += u32::from(*v);
                    }
                }
            }
        }
        for (out, sum) in out_row.iter_mut().zip(&sums) {
            *out = ((sum + area / 2) / area) as u8;
        }
    });
    out
}

/// For each destination index, the source indices it covers and their share
/// of its area (the shares sum to 1).
fn coverage(source: usize, dest: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = source as f64 / dest as f64;
    (0..dest)
        .map(|i| {
            let (start, end) = (
                i as f64 * scale,
                ((i + 1) as f64 * scale).min(source as f64),
            );
            (start.floor() as usize..(end.ceil() as usize).min(source))
                .map(|j| {
                    let covered = end.min(j as f64 + 1.0) - start.max(j as f64);
                    (j, (covered / scale) as f32)
                })
                .filter(|(_, weight)| *weight > 0.0)
                .collect()
        })
        .collect()
}

/// Area-weighted shrink of a `size` image (rows `stride` bytes apart) to
/// `target`, for scale factors from 1 to about 2.
fn area_resize(
    src: &[u8],
    stride: usize,
    bpp: usize,
    size: (usize, usize),
    target: (usize, usize),
) -> Vec<u8> {
    let (columns, rows) = (coverage(size.0, target.0), coverage(size.1, target.1));
    let row_len = target.0 * bpp;
    let mut out = vec![0u8; row_len * target.1];
    fill_rows(&mut out, row_len, |y, out_row| {
        let mut sums = vec![0f32; row_len];
        for &(sy, row_weight) in &rows[y] {
            let row = &src[sy * stride..sy * stride + size.0 * bpp];
            for (sum, sources) in sums.chunks_exact_mut(bpp).zip(&columns) {
                for &(sx, weight) in sources {
                    let weight = weight * row_weight;
                    for (s, v) in sum.iter_mut().zip(&row[sx * bpp..sx * bpp + bpp]) {
                        *s += weight * f32::from(*v);
                    }
                }
            }
        }
        for (out, sum) in out_row.iter_mut().zip(&sums) {
            *out = sum.round().clamp(0.0, 255.0) as u8;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_images_well_beyond_the_screen_get_a_copy() {
        assert_eq!(display_size(7680, 4320, (3840, 2400)), Some((3840, 2160)));
        assert_eq!(display_size(4000, 16000, (2880, 1800)), Some((450, 1800)));
        // Within about 13% of the screen, the original is shown.
        assert_eq!(display_size(3200, 2000, (2880, 1800)), None);
        assert_eq!(display_size(1920, 1080, (2880, 1800)), None);
        assert_eq!(display_size(0, 10, (2880, 1800)), None);
    }

    #[test]
    fn downscale_keeps_layout_colours_and_ignores_row_padding() {
        // 40×20 RGBA with 8 bytes of padding per row: left half red, right blue.
        let (w, h, bpp) = (40u32, 20u32, 4usize);
        let stride = w as usize * bpp + 8;
        let mut src = vec![0xEEu8; stride * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let pixel = if x < 20 {
                    [200, 0, 0, 255]
                } else {
                    [0, 0, 200, 255]
                };
                src[y * stride + x * bpp..][..4].copy_from_slice(&pixel);
            }
        }
        for target in [(10, 5), (13, 7)] {
            let out = downscale(&src, w, h, stride, bpp, target);
            assert_eq!(out.len(), (target.0 * target.1) as usize * bpp);
            let at = |x: u32, y: u32| &out[((y * target.0 + x) as usize) * bpp..][..4];
            assert_eq!(at(0, 0), &[200, 0, 0, 255], "{target:?}");
            assert_eq!(
                at(target.0 - 1, target.1 - 1),
                &[0, 0, 200, 255],
                "{target:?}"
            );
        }
    }

    #[test]
    fn downscale_handles_three_and_one_byte_pixels() {
        let rgb = vec![90u8; 30 * 30 * 3];
        assert_eq!(
            downscale(&rgb, 30, 30, 90, 3, (7, 7)),
            vec![90u8; 7 * 7 * 3]
        );
        let gray: Vec<u8> = (0..64).map(|i| if i % 2 == 0 { 0 } else { 255 }).collect();
        // Alternating columns average to mid-grey.
        let out = downscale(&gray, 8, 8, 8, 1, (4, 4));
        assert!(out.iter().all(|v| (127..=128).contains(v)), "{out:?}");
    }
}
