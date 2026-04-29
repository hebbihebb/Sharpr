/// Compute Laplacian variance of an RGBA thumbnail buffer.
///
/// The variance of the Laplacian response is a reliable focus measure —
/// a sharp image has many strong edges (high variance); a blurry one does not.
/// Running this on the thumbnail-sized buffer (~160 px tall) takes ~2 ms.
pub fn laplacian_variance(rgba: &[u8], width: u32, height: u32) -> f64 {
    let w = width as usize;
    let h = height as usize;
    if w < 3 || h < 3 || rgba.len() < w * h * 4 {
        return 0.0;
    }

    let luma: Vec<f32> = rgba
        .chunks_exact(4)
        .map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
        .collect();

    // 3×3 Laplacian kernel: [0,1,0 / 1,-4,1 / 0,1,0]
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let n = ((w - 2) * (h - 2)) as f64;

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let c = luma[y * w + x];
            let lap = luma[(y - 1) * w + x]
                + luma[(y + 1) * w + x]
                + luma[y * w + (x + 1)]
                + luma[y * w + (x - 1)]
                - 4.0 * c;
            let v = lap as f64;
            sum += v;
            sum_sq += v * v;
        }
    }

    let mean = sum / n;
    sum_sq / n - mean * mean
}

/// Map raw Laplacian variance to a 0.0–1.0 sharpness score.
///
/// Reference points on the log scale:
///   variance ~10  → 0.18  (visibly out-of-focus)
///   variance ~100 → 0.61  (acceptable, slightly soft)
///   variance ~500 → 0.90  (sharp)
///   variance ~800 → 1.0   (very sharp)
pub fn normalize_sharpness(variance: f64) -> f64 {
    if variance <= 0.0 {
        return 0.0;
    }
    ((variance.ln() - 1.0) / 5.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_colour_is_zero_variance() {
        let pixel = [128u8, 64, 32, 255];
        let rgba: Vec<u8> = pixel.iter().copied().cycle().take(10 * 10 * 4).collect();
        assert_eq!(laplacian_variance(&rgba, 10, 10), 0.0);
    }

    #[test]
    fn checkerboard_has_high_variance() {
        let mut rgba = vec![0u8; 10 * 10 * 4];
        for y in 0..10usize {
            for x in 0..10usize {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                let i = (y * 10 + x) * 4;
                rgba[i] = v;
                rgba[i + 1] = v;
                rgba[i + 2] = v;
                rgba[i + 3] = 255;
            }
        }
        assert!(laplacian_variance(&rgba, 10, 10) > 1000.0);
    }

    #[test]
    fn normalize_maps_typical_range() {
        assert!(normalize_sharpness(0.0) == 0.0);
        assert!(normalize_sharpness(10.0) > 0.0);
        assert!(normalize_sharpness(500.0) > 0.8);
        assert!(normalize_sharpness(f64::MAX) == 1.0);
    }
}
