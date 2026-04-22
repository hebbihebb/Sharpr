use image::RgbImage;

pub struct TileConfig {
    /// Size of each tile's core (non-overlapping) region in input pixels.
    pub tile_size: usize,
    /// Extra pixels added on each side when extracting a tile; discarded after
    /// upscaling to hide edge artifacts from the model's sliding-window attention.
    pub overlap: usize,
    /// Scale factor the tile processor produces (4 for Swin2SR).
    pub scale: usize,
}

impl Default for TileConfig {
    fn default() -> Self {
        Self {
            tile_size: 256,
            overlap: 16,
            scale: 4,
        }
    }
}

/// Splits `img` into overlapping tiles, runs each tile through `process_tile`,
/// stitches the results, and returns the full upscaled image.
///
/// `on_progress` is called with a fraction in [0, 1] after each tile completes.
/// `process_tile` must return an image of exactly `scale × (tile_w, tile_h)`.
pub fn process_tiled<F, P>(
    img: &RgbImage,
    config: &TileConfig,
    mut on_progress: P,
    mut process_tile: F,
) -> Result<RgbImage, String>
where
    F: FnMut(RgbImage) -> Result<RgbImage, String>,
    P: FnMut(f32),
{
    let img_w = img.width() as usize;
    let img_h = img.height() as usize;
    let TileConfig {
        tile_size,
        overlap,
        scale,
    } = *config;

    let cols = img_w.div_ceil(tile_size);
    let rows = img_h.div_ceil(tile_size);
    let total = (cols * rows) as f32;

    let mut output = RgbImage::new((img_w * scale) as u32, (img_h * scale) as u32);

    for (idx, (row, col)) in (0..rows)
        .flat_map(|r| (0..cols).map(move |c| (r, c)))
        .enumerate()
    {
        // Core region in the source image (may be smaller at right/bottom edges).
        let core_x0 = col * tile_size;
        let core_y0 = row * tile_size;
        let core_x1 = (core_x0 + tile_size).min(img_w);
        let core_y1 = (core_y0 + tile_size).min(img_h);

        // Padded extraction region (clamped to image bounds).
        let pad_x0 = core_x0.saturating_sub(overlap);
        let pad_y0 = core_y0.saturating_sub(overlap);
        let pad_x1 = (core_x1 + overlap).min(img_w);
        let pad_y1 = (core_y1 + overlap).min(img_h);
        let pad_w = pad_x1 - pad_x0;
        let pad_h = pad_y1 - pad_y0;

        let tile = image::imageops::crop_imm(
            img,
            pad_x0 as u32,
            pad_y0 as u32,
            pad_w as u32,
            pad_h as u32,
        )
        .to_image();

        let tile_out = process_tile(tile)?;

        let expected_w = (pad_w * scale) as u32;
        let expected_h = (pad_h * scale) as u32;
        if tile_out.width() != expected_w || tile_out.height() != expected_h {
            return Err(format!(
                "Tile ({col},{row}) output {}×{} ≠ expected {expected_w}×{expected_h}",
                tile_out.width(),
                tile_out.height()
            ));
        }

        // Crop out the overlap margins from the upscaled tile to get the core output.
        let crop_x0 = ((core_x0 - pad_x0) * scale) as u32;
        let crop_y0 = ((core_y0 - pad_y0) * scale) as u32;
        let crop_w = ((core_x1 - core_x0) * scale) as u32;
        let crop_h = ((core_y1 - core_y0) * scale) as u32;

        let core_out =
            image::imageops::crop_imm(&tile_out, crop_x0, crop_y0, crop_w, crop_h).to_image();

        image::imageops::replace(
            &mut output,
            &core_out,
            (core_x0 * scale) as i64,
            (core_y0 * scale) as i64,
        );

        on_progress((idx + 1) as f32 / total);
    }

    Ok(output)
}
