//! Shared preview decode pipeline for viewer loads and prefetch work.
//! It tries three decode strategies in priority order: embedded EXIF preview,
//! turbojpeg scaled JPEG decode, then a full `image::ImageReader` fallback.
//! EXIF orientation is applied consistently in all three paths before pixels
//! are returned to the caller.
//! Callers should use `decode_preview(path, mode)` and handle
//! `PreviewDecodeError` variants to decide how the UI should recover.

pub mod worker;

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
    ScaledWebp,
    ScaledJxl,
    FullDecode,
}

#[derive(Debug)]
pub struct PreviewImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source: PreviewSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewDecodeError {
    /// File could not be opened (missing, permission denied, I/O error).
    OpenFailed,
    /// Image format could not be detected from the file contents.
    FormatDetectFailed,
    /// Decoder reported an error (corrupt file, truncated data, etc.).
    DecodeFailed,
    /// Format is recognised but not supported by any decode path.
    Unsupported,
    /// Decoded image reports zero width or height.
    InvalidDimensions,
}

impl PreviewDecodeError {
    /// Short human-readable label suitable for logging and bench events.
    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenFailed => "open_failed",
            Self::FormatDetectFailed => "format_detect_failed",
            Self::DecodeFailed => "decode_failed",
            Self::Unsupported => "unsupported",
            Self::InvalidDimensions => "invalid_dimensions",
        }
    }
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

    if is_webp_path(path) {
        if let Some(img) = decode_webp_rgba_scaled(path, MIN_VIEWER_LONG_EDGE as u32) {
            crate::bench_event!(
                "preview.decode.finish",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "mode": preview_mode_label(mode),
                    "source": "scaled_webp",
                    "width": img.width,
                    "height": img.height,
                }),
            );
            return Ok(img);
        }
    }

    if crate::jxl::is_jxl_path(path) {
        if let Some(img) = decode_jxl_rgba_scaled(path, MIN_VIEWER_LONG_EDGE as u32) {
            crate::bench_event!(
                "preview.decode.finish",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "mode": preview_mode_label(mode),
                    "source": "scaled_jxl",
                    "width": img.width,
                    "height": img.height,
                }),
            );
            return Ok(img);
        }
    }

    let img = apply_exif_orientation(decode_full_image(path)?, path);
    let rgba = img.into_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        crate::bench_event!(
            "preview.decode.fail",
            serde_json::json!({
                "path": path.display().to_string(),
                "reason": "invalid_dimensions",
            }),
        );
        return Err(PreviewDecodeError::InvalidDimensions);
    }
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

    let mut previews = metadata.get_preview_images()?;
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

fn decode_full_image(path: &Path) -> Result<image::DynamicImage, PreviewDecodeError> {
    if crate::jxl::is_jxl_path(path) {
        return crate::jxl::decode_path(path).map_err(|e| {
            crate::bench_event!(
                "preview.decode.fail",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "reason": "decode_failed",
                    "error": e,
                }),
            );
            PreviewDecodeError::DecodeFailed
        });
    }

    let file = File::open(path).map_err(|e| {
        crate::bench_event!(
            "preview.decode.fail",
            serde_json::json!({
                "path": path.display().to_string(),
                "reason": "open_failed",
                "error": e.to_string(),
            }),
        );
        PreviewDecodeError::OpenFailed
    })?;
    let reader = image::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| {
            crate::bench_event!(
                "preview.decode.fail",
                serde_json::json!({
                    "path": path.display().to_string(),
                    "reason": "format_detect_failed",
                    "error": e.to_string(),
                }),
            );
            PreviewDecodeError::FormatDetectFailed
        })?;
    reader.decode().map_err(|e| {
        crate::bench_event!(
            "preview.decode.fail",
            serde_json::json!({
                "path": path.display().to_string(),
                "reason": "decode_failed",
                "error": e.to_string(),
            }),
        );
        PreviewDecodeError::DecodeFailed
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

fn is_webp_path(path: &Path) -> bool {
    let by_extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "webp"))
        .unwrap_or(false);
    if by_extension {
        return true;
    }

    let mut file = std::fs::File::open(path).ok();
    let Some(file) = file.as_mut() else {
        return false;
    };
    let mut magic = [0u8; 12];
    std::io::Read::read_exact(file, &mut magic).is_ok()
        && &magic[0..4] == b"RIFF"
        && &magic[8..12] == b"WEBP"
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

fn decode_webp_rgba_scaled(path: &Path, min_long_edge: u32) -> Option<PreviewImage> {
    let webp_data = std::fs::read(path).ok()?;
    let mut config = libwebp_sys::WebPDecoderConfig::new().ok()?;
    let status = unsafe {
        libwebp_sys::WebPGetFeatures(webp_data.as_ptr(), webp_data.len(), &mut config.input)
    };
    if status != libwebp_sys::VP8StatusCode::VP8_STATUS_OK {
        return None;
    }
    if config.input.has_animation != 0 {
        return None;
    }

    let src_width = u32::try_from(config.input.width).ok()?;
    let src_height = u32::try_from(config.input.height).ok()?;
    let (target_width, target_height) =
        choose_scaled_dimensions(src_width, src_height, min_long_edge);
    let stride = usize::try_from(target_width).ok()?.checked_mul(4)?;
    let mut rgba = vec![0u8; stride.checked_mul(usize::try_from(target_height).ok()?)?];

    config.options.use_threads = 1;
    let use_scaling = target_width != src_width || target_height != src_height;
    config.options.use_scaling = i32::from(use_scaling);
    config.options.scaled_width = i32::try_from(target_width).ok()?;
    config.options.scaled_height = i32::try_from(target_height).ok()?;
    config.output.colorspace = libwebp_sys::WEBP_CSP_MODE::MODE_RGBA;
    config.output.width = i32::try_from(target_width).ok()?;
    config.output.height = i32::try_from(target_height).ok()?;
    config.output.is_external_memory = 1;
    config.output.u.RGBA.rgba = rgba.as_mut_ptr();
    config.output.u.RGBA.stride = i32::try_from(stride).ok()?;
    config.output.u.RGBA.size = rgba.len();

    let status =
        unsafe { libwebp_sys::WebPDecode(webp_data.as_ptr(), webp_data.len(), &mut config) };
    if status != libwebp_sys::VP8StatusCode::VP8_STATUS_OK {
        return None;
    }

    let image = image::RgbaImage::from_raw(target_width, target_height, rgba)?;
    let image = apply_exif_orientation(image::DynamicImage::ImageRgba8(image), path).into_rgba8();
    Some(PreviewImage {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
        source: PreviewSource::ScaledWebp,
    })
}

fn decode_jxl_rgba_scaled(path: &Path, min_long_edge: u32) -> Option<PreviewImage> {
    let img = crate::jxl::decode_path(path).ok()?;
    let img = apply_exif_orientation(img, path);
    let (target_width, target_height) =
        choose_scaled_dimensions(img.width(), img.height(), min_long_edge);
    let img = if target_width != img.width() || target_height != img.height() {
        img.resize(
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let rgba = img.into_rgba8();
    Some(PreviewImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        source: PreviewSource::ScaledJxl,
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
        if factor.scale(width).max(factor.scale(height)) >= min_long_edge {
            return factor;
        }
    }

    turbojpeg::ScalingFactor::ONE
}

pub(crate) fn choose_scaled_dimensions(width: u32, height: u32, min_long_edge: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (width, height);
    }

    let long_edge = width.max(height);
    if long_edge <= min_long_edge {
        return (width, height);
    }

    let scale = min_long_edge as f64 / long_edge as f64;
    let scaled_width = ((width as f64) * scale).round().max(1.0) as u32;
    let scaled_height = ((height as f64) * scale).round().max(1.0) as u32;
    (scaled_width, scaled_height)
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

    #[test]
    fn decode_missing_file_returns_open_failed() {
        let result = decode_preview(
            std::path::Path::new("/nonexistent/does_not_exist.jpg"),
            PreviewDecodeMode::Viewer,
        );
        assert!(matches!(result, Err(PreviewDecodeError::OpenFailed)));
    }

    #[test]
    fn choose_scaled_dimensions_reduces_large_images_to_target_long_edge() {
        let (w, h) = choose_scaled_dimensions(6000, 4000, 1280);
        assert_eq!((w, h), (1280, 853));
    }

    #[test]
    fn choose_scaled_dimensions_keeps_smaller_images_at_full_size() {
        let (w, h) = choose_scaled_dimensions(1200, 800, 1280);
        assert_eq!((w, h), (1200, 800));
    }

    #[test]
    fn error_labels_are_distinct() {
        let errors = [
            PreviewDecodeError::OpenFailed,
            PreviewDecodeError::FormatDetectFailed,
            PreviewDecodeError::DecodeFailed,
            PreviewDecodeError::Unsupported,
            PreviewDecodeError::InvalidDimensions,
        ];
        let labels: Vec<_> = errors.iter().map(|e| e.label()).collect();
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(
            labels.len(),
            unique.len(),
            "all error labels must be distinct"
        );
    }
}
