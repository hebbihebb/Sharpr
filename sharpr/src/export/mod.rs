//! Export pipeline for writing user-visible copies without overwriting sources.
//! `export_image` decodes the source with EXIF orientation applied, optionally
//! downscales with Lanczos3, then writes into the destination directory.
//! Supported output formats are JPEG, PNG, and WebP; WebP is always lossless in
//! the current implementation.
//! `unique_output_path` guarantees that exports never overwrite an existing
//! file in the destination folder.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;

use crate::metadata::orientation::apply_exif_orientation;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Jpeg,
    Png,
    Webp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFolderKind {
    Upscaled,
    Export,
}

impl OutputFolderKind {
    pub fn folder_name(self) -> &'static str {
        match self {
            Self::Upscaled => "Upscaled",
            Self::Export => "Export",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub destination: PathBuf,
    /// Longest edge of the output image; `None` means no resize.
    pub max_edge: Option<u32>,
    pub format: ExportFormat,
    /// JPEG quality, 1–100. Ignored for PNG and WebP (both written losslessly).
    pub quality: u8,
    pub filename_suffix: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct ExportResult {
    pub source: PathBuf,
    pub output: PathBuf,
}

#[derive(Debug)]
pub enum ExportError {
    Decode(String),
    Encode(String),
    Io(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(s) => write!(f, "decode error: {s}"),
            Self::Encode(s) => write!(f, "encode error: {s}"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Decode `source`, resize when needed, and write to `config.destination`.
///
/// Generates a unique output file name so existing files are never overwritten.
pub fn export_image(source: &Path, config: &ExportConfig) -> Result<ExportResult, ExportError> {
    let img = {
        let file = std::fs::File::open(source)
            .map_err(|e| ExportError::Io(format!("open {}: {e}", source.display())))?;
        let reader = image::ImageReader::new(std::io::BufReader::new(file))
            .with_guessed_format()
            .map_err(|e| ExportError::Decode(e.to_string()))?;
        let decoded = reader
            .decode()
            .map_err(|e| ExportError::Decode(e.to_string()))?;
        apply_exif_orientation(decoded, source)
    };

    let img = resize_if_needed(img, config.max_edge);

    let output = match config.filename_suffix.as_deref() {
        Some(suffix) => unique_output_path_with_suffix(
            &config.destination,
            source,
            suffix,
            format_extension(config.format),
        ),
        None => unique_output_path(&config.destination, source, config.format),
    };

    save_image(&img, &output, config.format, config.quality)?;

    Ok(ExportResult {
        source: source.to_path_buf(),
        output,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return a path in `dest_dir` that does not yet exist.
///
/// Tries `<stem>.<ext>`, then `<stem>-1.<ext>`, `<stem>-2.<ext>`, … up to 999.
pub(crate) fn unique_output_path(dest_dir: &Path, source: &Path, format: ExportFormat) -> PathBuf {
    unique_output_path_for_extension(dest_dir, source, format_extension(format))
}

/// Return the preferred root for user-facing exported images.
pub fn pictures_root_dir() -> PathBuf {
    dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Pictures")))
        .unwrap_or_else(|| PathBuf::from("./Pictures"))
}

/// Return the built-in output folder for the given conversion kind.
pub fn default_output_dir(kind: OutputFolderKind) -> PathBuf {
    pictures_root_dir().join(kind.folder_name())
}

/// Resolve a configured output folder or fall back to the built-in one.
pub fn resolve_output_dir(custom: Option<&PathBuf>, kind: OutputFolderKind) -> PathBuf {
    custom.cloned().unwrap_or_else(|| default_output_dir(kind))
}

/// Build the suffix used for downscale/export output names.
pub fn export_filename_suffix(max_edge: Option<u32>, format: ExportFormat) -> String {
    let size = max_edge
        .map(|edge| format!("{edge}px"))
        .unwrap_or_else(|| "original".to_string());
    format!("export-{size}-{}", format_extension(format))
}

/// Return a unique output path in `dest_dir` with an explicit file extension.
pub fn unique_output_path_for_extension(dest_dir: &Path, source: &Path, ext: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    unique_output_path_for_stem(dest_dir, &stem, ext)
}

/// Return a unique output path in `dest_dir` using a human-readable suffix.
pub fn unique_output_path_with_suffix(
    dest_dir: &Path,
    source: &Path,
    suffix: &str,
    ext: &str,
) -> PathBuf {
    let stem = source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let stem = format!("{stem}-{}", sanitize_suffix(suffix));
    unique_output_path_for_stem(dest_dir, &stem, ext)
}

fn unique_output_path_for_stem(dest_dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = path_for_stem_and_ext(dest_dir, stem, ext);
    if !first.exists() {
        return first;
    }
    for n in 1..=999u32 {
        let candidate = path_for_stem_and_ext(dest_dir, &format!("{stem}-{n}"), ext);
        if !candidate.exists() {
            return candidate;
        }
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    path_for_stem_and_ext(dest_dir, &format!("{stem}-{ts}"), ext)
}

fn path_for_stem_and_ext(dest_dir: &Path, stem: &str, ext: &str) -> PathBuf {
    if ext.is_empty() {
        dest_dir.join(stem)
    } else {
        dest_dir.join(format!("{stem}.{ext}"))
    }
}

fn sanitize_suffix(suffix: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in suffix.chars() {
        let normalized = match ch {
            'A'..='Z' => ch.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' => ch,
            _ => '-',
        };
        if normalized == '-' {
            if !out.is_empty() && !last_dash {
                out.push('-');
            }
            last_dash = true;
        } else {
            out.push(normalized);
            last_dash = false;
        }
    }
    out.trim_matches('-').to_string()
}

/// Decode `source`, resize when needed, and write to the explicit `output` path.
///
/// Unlike `export_image`, the caller controls the output file name; no
/// uniqueness suffix is added.
pub fn export_to_path(
    source: &Path,
    output: &Path,
    max_edge: Option<u32>,
    format: ExportFormat,
    quality: u8,
) -> Result<(), ExportError> {
    let img = {
        let file = std::fs::File::open(source)
            .map_err(|e| ExportError::Io(format!("open {}: {e}", source.display())))?;
        let reader = image::ImageReader::new(std::io::BufReader::new(file))
            .with_guessed_format()
            .map_err(|e| ExportError::Decode(e.to_string()))?;
        let decoded = reader
            .decode()
            .map_err(|e| ExportError::Decode(e.to_string()))?;
        apply_exif_orientation(decoded, source)
    };
    let img = resize_if_needed(img, max_edge);
    save_image(&img, output, format, quality)
}

/// Downscale `img` so its longest edge is at most `max_edge`. No-op when the
/// image already fits, or when `max_edge` is `None`.
pub(crate) fn resize_if_needed(
    img: image::DynamicImage,
    max_edge: Option<u32>,
) -> image::DynamicImage {
    let Some(limit) = max_edge else {
        return img;
    };
    let long = img.width().max(img.height());
    if long <= limit {
        return img;
    }
    let (nw, nh) = if img.width() >= img.height() {
        let nh = (img.height() as f64 * limit as f64 / img.width() as f64).round() as u32;
        (limit, nh.max(1))
    } else {
        let nw = (img.width() as f64 * limit as f64 / img.height() as f64).round() as u32;
        (nw.max(1), limit)
    };
    img.resize_exact(nw, nh, FilterType::Lanczos3)
}

pub fn format_extension(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Jpeg => "jpg",
        ExportFormat::Png => "png",
        ExportFormat::Webp => "webp",
    }
}

fn save_image(
    img: &image::DynamicImage,
    output: &Path,
    format: ExportFormat,
    quality: u8,
) -> Result<(), ExportError> {
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
    use image::codecs::webp::WebPEncoder;
    use image::{ExtendedColorType, ImageEncoder};
    use std::io::BufWriter;

    let file = std::fs::File::create(output)
        .map_err(|e| ExportError::Io(format!("create {}: {e}", output.display())))?;
    let writer = BufWriter::new(file);

    match format {
        ExportFormat::Jpeg => {
            let rgb = img.to_rgb8();
            JpegEncoder::new_with_quality(writer, quality.clamp(1, 100))
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|e| ExportError::Encode(e.to_string()))
        }
        ExportFormat::Png => {
            let rgba = img.to_rgba8();
            PngEncoder::new_with_quality(writer, CompressionType::Default, PngFilter::Adaptive)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|e| ExportError::Encode(e.to_string()))
        }
        ExportFormat::Webp => {
            if img.color().has_alpha() {
                let rgba = img.to_rgba8();
                WebPEncoder::new_lossless(writer)
                    .write_image(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(|e| ExportError::Encode(e.to_string()))
            } else {
                let rgb = img.to_rgb8();
                WebPEncoder::new_lossless(writer)
                    .write_image(
                        rgb.as_raw(),
                        rgb.width(),
                        rgb.height(),
                        ExtendedColorType::Rgb8,
                    )
                    .map_err(|e| ExportError::Encode(e.to_string()))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("sharpr-export-test-{tag}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn unique_path_returns_stem_ext_when_free() {
        let dir = temp_dir("free");
        let src = PathBuf::from("/photos/vacation.jpg");
        let out = unique_output_path(&dir, &src, ExportFormat::Jpeg);
        assert_eq!(out, dir.join("vacation.jpg"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_path_increments_when_taken() {
        let dir = temp_dir("taken");
        let src = PathBuf::from("/photos/shot.jpg");
        std::fs::write(dir.join("shot.jpg"), b"").unwrap();
        let out = unique_output_path(&dir, &src, ExportFormat::Jpeg);
        assert_eq!(out, dir.join("shot-1.jpg"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_path_uses_format_extension() {
        let dir = temp_dir("ext");
        let src = PathBuf::from("/photos/shot.jpg");
        let out = unique_output_path(&dir, &src, ExportFormat::Png);
        assert_eq!(out, dir.join("shot.png"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_path_with_suffix_appends_human_label() {
        let dir = temp_dir("suffix");
        let src = PathBuf::from("/photos/shot.jpg");
        let out = unique_output_path_with_suffix(&dir, &src, "upscaled-comfyui-standard", "png");
        assert_eq!(out, dir.join("shot-upscaled-comfyui-standard.png"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_path_with_suffix_still_increments() {
        let dir = temp_dir("suffix-taken");
        let src = PathBuf::from("/photos/shot.jpg");
        std::fs::write(dir.join("shot-export-1920px-jpg.jpg"), b"").unwrap();
        let out = unique_output_path_with_suffix(&dir, &src, "export-1920px-jpg", "jpg");
        assert_eq!(out, dir.join("shot-export-1920px-jpg-1.jpg"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn export_suffix_includes_size_and_format() {
        assert_eq!(
            export_filename_suffix(Some(1920), ExportFormat::Jpeg),
            "export-1920px-jpg"
        );
        assert_eq!(
            export_filename_suffix(None, ExportFormat::Png),
            "export-original-png"
        );
    }

    #[test]
    fn resolve_output_dir_prefers_custom_path() {
        let custom = PathBuf::from("/tmp/custom-export");
        let resolved = resolve_output_dir(Some(&custom), OutputFolderKind::Export);
        assert_eq!(resolved, custom);
    }

    #[test]
    fn default_output_dir_uses_expected_folder_names() {
        assert_eq!(
            default_output_dir(OutputFolderKind::Upscaled)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("Upscaled")
        );
        assert_eq!(
            default_output_dir(OutputFolderKind::Export)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("Export")
        );
    }

    #[test]
    fn resize_if_needed_no_op_when_fits() {
        let img = image::DynamicImage::new_rgb8(800, 600);
        let out = resize_if_needed(img, Some(1024));
        assert_eq!(out.width(), 800);
        assert_eq!(out.height(), 600);
    }

    #[test]
    fn resize_if_needed_no_op_when_none() {
        let img = image::DynamicImage::new_rgb8(4000, 3000);
        let out = resize_if_needed(img, None);
        assert_eq!(out.width(), 4000);
    }

    #[test]
    fn resize_if_needed_scales_landscape() {
        let img = image::DynamicImage::new_rgb8(4000, 2000);
        let out = resize_if_needed(img, Some(2000));
        assert_eq!(out.width(), 2000);
        assert_eq!(out.height(), 1000);
    }

    #[test]
    fn resize_if_needed_scales_portrait() {
        let img = image::DynamicImage::new_rgb8(2000, 4000);
        let out = resize_if_needed(img, Some(2000));
        assert_eq!(out.width(), 1000);
        assert_eq!(out.height(), 2000);
    }
}
