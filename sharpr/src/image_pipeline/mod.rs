use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::metadata::orientation::apply_exif_orientation;

const MIN_PREVIEW_LONG_EDGE: u32 = 1024;
const MIN_VIEWER_LONG_EDGE: usize = 1280;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewDecodeMode {
    Viewer,
    Prefetch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewSource {
    EmbeddedPreview,
    ScaledJpeg,
    FullDecode,
}

#[derive(Debug)]
pub struct PreviewImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source: PreviewSource,
}

#[derive(Debug)]
pub enum PreviewDecodeError {
    DecodeFailed,
}

pub fn decode_preview(
    path: &Path,
    mode: PreviewDecodeMode,
) -> Result<PreviewImage, PreviewDecodeError> {
    if let Some(img) = decode_embedded_preview(path) {
        crate::bench_event!(
            "preview.decode.finish",
            serde_json::json!({
                "path": path.display().to_string(),
                "mode": preview_mode_label(mode),
                "source": "embedded_preview",
                "width": img.width,
                "height": img.height,
            }),
        );
        return Ok(img);
    }

    if is_jpeg_path(path) {
        if let Some(img) = decode_jpeg_rgba_scaled(path) {
            crate::bench_event!(
                "preview.decode.finish",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "mode": preview_mode_label(mode),
                    "source": "scaled_jpeg",
                    "width": img.width,
                    "height": img.height,
                }),
            );
            return Ok(img);
        }
    }

    let file = File::open(path).map_err(|_| PreviewDecodeError::DecodeFailed)?;
    let reader = image::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|_| PreviewDecodeError::DecodeFailed)?;
    let img = apply_exif_orientation(
        reader
            .decode()
            .map_err(|_| PreviewDecodeError::DecodeFailed)?,
        path,
    );
    let rgba = img.into_rgba8();
    let decoded = PreviewImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        source: PreviewSource::FullDecode,
    };

    crate::bench_event!(
        "preview.decode.finish",
        serde_json::json!({
            "path": path.display().to_string(),
            "mode": preview_mode_label(mode),
            "source": "full_decode",
            "width": decoded.width,
            "height": decoded.height,
        }),
    );

    Ok(decoded)
}

fn decode_embedded_preview(path: &Path) -> Option<PreviewImage> {
    let metadata = {
        let _guard = crate::metadata::exif::rexiv2_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        rexiv2::Metadata::new_from_path(path)
    }
    .ok()?;

    let mut previews = metadata.get_preview_images().ok()?;
    previews.sort_by_key(|preview| preview.get_width().max(preview.get_height()));

    let preview = previews.into_iter().rev().find(|preview| {
        preview.get_width().max(preview.get_height()) >= MIN_PREVIEW_LONG_EDGE
            && matches!(preview.get_media_type(), Ok(rexiv2::MediaType::Jpeg))
    })?;

    let img =
        image::load_from_memory_with_format(&preview.get_data().ok()?, image::ImageFormat::Jpeg)
            .ok()?;

    let rgba = apply_exif_orientation(img, path).into_rgba8();
    Some(PreviewImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        source: PreviewSource::EmbeddedPreview,
    })
}

fn is_jpeg_path(path: &Path) -> bool {
    let by_extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
        .unwrap_or(false);
    if by_extension {
        return true;
    }

    let mut file = std::fs::File::open(path).ok();
    let Some(file) = file.as_mut() else {
        return false;
    };
    let mut magic = [0u8; 2];
    std::io::Read::read_exact(file, &mut magic).is_ok() && magic == [0xFF, 0xD8]
}

fn decode_jpeg_rgba_scaled(path: &Path) -> Option<PreviewImage> {
    let jpeg_data = std::fs::read(path).ok()?;
    let mut decompressor = turbojpeg::Decompressor::new().ok()?;
    let header = decompressor.read_header(&jpeg_data).ok()?;
    let scale = choose_jpeg_scale_factor(&header, MIN_VIEWER_LONG_EDGE);
    let scaled = header.scaled(scale);
    let pitch = scaled.width * turbojpeg::PixelFormat::RGBA.size();
    let mut image = turbojpeg::Image {
        pixels: vec![0; pitch * scaled.height],
        width: scaled.width,
        pitch,
        height: scaled.height,
        format: turbojpeg::PixelFormat::RGBA,
    };

    decompressor.set_scaling_factor(scale).ok()?;
    decompressor
        .decompress(&jpeg_data, image.as_deref_mut())
        .ok()?;

    let rgba = image::RgbaImage::from_raw(scaled.width as u32, scaled.height as u32, image.pixels)?;
    let img = apply_exif_orientation(image::DynamicImage::ImageRgba8(rgba), path).into_rgba8();

    Some(PreviewImage {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
        source: PreviewSource::ScaledJpeg,
    })
}

fn choose_jpeg_scale_factor(
    header: &turbojpeg::DecompressHeader,
    min_long_edge: usize,
) -> turbojpeg::ScalingFactor {
    choose_jpeg_scale_factor_for_dims(
        header.width,
        header.height,
        header.is_lossless,
        min_long_edge,
    )
}

fn choose_jpeg_scale_factor_for_dims(
    width: usize,
    height: usize,
    is_lossless: bool,
    min_long_edge: usize,
) -> turbojpeg::ScalingFactor {
    if is_lossless {
        return turbojpeg::ScalingFactor::ONE;
    }

    let candidates = [
        turbojpeg::ScalingFactor::ONE_EIGHTH,
        turbojpeg::ScalingFactor::ONE_QUARTER,
        turbojpeg::ScalingFactor::ONE_HALF,
    ];

    for factor in candidates {
        let scaled = factor.scale(width, height);
        if scaled.0.max(scaled.1) >= min_long_edge {
            return factor;
        }
    }

    turbojpeg::ScalingFactor::ONE
}

fn preview_mode_label(mode: PreviewDecodeMode) -> &'static str {
    match mode {
        PreviewDecodeMode::Viewer => "viewer",
        PreviewDecodeMode::Prefetch => "prefetch",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_scale_prefers_smallest_factor_that_meets_target() {
        let factor = choose_jpeg_scale_factor_for_dims(6000, 4000, false, 1280);
        assert_eq!(factor, turbojpeg::ScalingFactor::ONE_QUARTER);
    }

    #[test]
    fn choose_scale_falls_back_to_full_when_needed() {
        let factor = choose_jpeg_scale_factor_for_dims(1800, 1200, false, 1280);
        assert_eq!(factor, turbojpeg::ScalingFactor::ONE);
    }

    #[test]
    fn choose_scale_keeps_lossless_at_full_size() {
        let factor = choose_jpeg_scale_factor_for_dims(6000, 4000, true, 1280);
        assert_eq!(factor, turbojpeg::ScalingFactor::ONE);
    }
}
