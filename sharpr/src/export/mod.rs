use std::path::{Path, PathBuf};
use std::time::Instant;

use image::imageops::FilterType;

use crate::metadata::orientation::apply_exif_orientation;

pub mod batch;

pub const WEBP_MAX_DIMENSION: u32 = 16_383;

/// One copy to write: the source, its folder, and the output size.
#[derive(Debug, Clone)]
struct CopyTarget {
    source: PathBuf,
    destination_dir: PathBuf,
    target_width: u32,
    target_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl CropRect {
    pub fn fits(self, (width, height): (u32, u32)) -> bool {
        self.width > 0
            && self.height > 0
            && self.x.checked_add(self.width).is_some_and(|v| v <= width)
            && self.y.checked_add(self.height).is_some_and(|v| v <= height)
    }
    pub fn centered_widescreen((width, height): (u32, u32)) -> Self {
        let units = (width / 16).min(height / 9);
        let (w, h) = if units > 0 {
            (units * 16, units * 9)
        } else {
            (width.max(1), height.max(1))
        };
        Self {
            x: width.saturating_sub(w) / 2,
            y: height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CropConfig {
    pub source: PathBuf,
    pub destination_dir: PathBuf,
    pub rectangle: CropRect,
}

/// Writes the cropped region as a lossless WebP copy (D13).
pub fn crop_copy(config: &CropConfig) -> Result<ResizeResult, ResizeError> {
    let target = CopyTarget {
        source: config.source.clone(),
        destination_dir: config.destination_dir.clone(),
        target_width: config.rectangle.width,
        target_height: config.rectangle.height,
    };
    copy_with_encoder(&target, Some(config.rectangle), "cropped", |image, path| {
        let bytes = webp_bytes(image, true, 100)?;
        write_flushed(path, &bytes)
    })
}

/// `image` as WebP: lossless, or lossy at `quality` (1–100). Images without
/// transparency are encoded without an alpha channel.
pub(crate) fn webp_bytes(
    image: &image::DynamicImage,
    lossless: bool,
    quality: u8,
) -> Result<Vec<u8>, ResizeError> {
    let quality = f32::from(quality.clamp(1, 100));
    let (width, height) = (image.width(), image.height());
    let encoded = if image.color().has_alpha() {
        webp::Encoder::from_rgba(image.to_rgba8().as_raw(), width, height)
            .encode_simple(lossless, quality)
    } else {
        webp::Encoder::from_rgb(image.to_rgb8().as_raw(), width, height)
            .encode_simple(lossless, quality)
    };
    encoded
        .map(|memory| memory.to_vec())
        .map_err(|error| ResizeError::Encode(format!("WebP encoding failed: {error:?}")))
}

/// Writes `bytes` to `path` and makes sure they reached the disk.
pub(crate) fn write_flushed(path: &Path, bytes: &[u8]) -> Result<(), ResizeError> {
    use std::io::Write;
    let mut file = std::fs::File::create(path).map_err(|e| ResizeError::Io(e.to_string()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| ResizeError::Io(e.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizeResult {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResizeError {
    Decode(String),
    InvalidDimensions,
    Encode(String),
    Io(String),
}

impl std::fmt::Display for ResizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(message) => write!(formatter, "decode failed: {message}"),
            Self::InvalidDimensions => {
                write!(formatter, "width and height must be greater than zero")
            }
            Self::Encode(message) => write!(formatter, "encoding failed: {message}"),
            Self::Io(message) => write!(formatter, "file operation failed: {message}"),
        }
    }
}

impl std::error::Error for ResizeError {}

pub fn oriented_source_dimensions(source: &Path) -> Result<(u32, u32), ResizeError> {
    crate::metadata::exif::display_dimensions(source).ok_or_else(|| {
        ResizeError::Decode(format!(
            "Could not read image dimensions: {}",
            source.display()
        ))
    })
}

pub fn oriented_dimensions((width, height): (u32, u32), orientation: u32) -> (u32, u32) {
    if matches!(orientation, 5..=8) {
        (height, width)
    } else {
        (width, height)
    }
}

fn copy_with_encoder<F>(
    config: &CopyTarget,
    crop: Option<CropRect>,
    label: &str,
    encoder: F,
) -> Result<ResizeResult, ResizeError>
where
    F: FnOnce(&image::DynamicImage, &Path) -> Result<(), ResizeError>,
{
    if config.target_width == 0 || config.target_height == 0 {
        return Err(ResizeError::InvalidDimensions);
    }
    if config.target_width > WEBP_MAX_DIMENSION || config.target_height > WEBP_MAX_DIMENSION {
        return Err(ResizeError::Encode(format!(
            "WebP supports at most {WEBP_MAX_DIMENSION} pixels per side; choose smaller dimensions"
        )));
    }

    let started = Instant::now();
    crate::bench_event!(
        "resize.copy.start",
        serde_json::json!({
            "source": config.source.display().to_string(),
            "destination": config.destination_dir.display().to_string(),
            "width": config.target_width,
            "height": config.target_height,
        }),
    );

    std::fs::create_dir_all(&config.destination_dir).map_err(|error| {
        ResizeError::Io(format!(
            "create {}: {error}",
            config.destination_dir.display()
        ))
    })?;

    let source = decode_oriented(&config.source)?;
    let source = if let Some(rect) = crop {
        if !rect.fits((source.width(), source.height())) {
            return Err(ResizeError::InvalidDimensions);
        }
        source.crop_imm(rect.x, rect.y, rect.width, rect.height)
    } else {
        source
    };
    let resized =
        if (source.width(), source.height()) == (config.target_width, config.target_height) {
            source
        } else {
            source.resize_exact(
                config.target_width,
                config.target_height,
                FilterType::Lanczos3,
            )
        };
    let (output, partial) = reserve_copy_paths(config, label)?;

    let result = encoder(&resized, &partial).and_then(|()| {
        std::fs::rename(&partial, &output).map_err(|error| {
            ResizeError::Io(format!(
                "move {} to {}: {error}",
                partial.display(),
                output.display()
            ))
        })
    });
    if let Err(error) = result {
        let _ = std::fs::remove_file(&partial);
        let _ = std::fs::remove_file(&output);
        crate::bench_error!(
            "resize.copy.fail",
            serde_json::json!({
                "source": config.source.display().to_string(),
                "error": error.to_string(),
            }),
        );
        return Err(error);
    }

    crate::bench_event!(
        "resize.copy.done",
        serde_json::json!({
            "source": config.source.display().to_string(),
            "output": output.display().to_string(),
            "duration_ms": crate::bench::duration_ms(started),
        }),
    );
    Ok(ResizeResult {
        output,
        width: config.target_width,
        height: config.target_height,
    })
}

/// The most memory one decode may take; the library has images of 270
/// megapixels and more, past the decoder's default 512 MB.
const MAX_DECODE_BYTES: u64 = 3 << 30;

/// The whole image in its displayed orientation. libjxl orients JPEG XL
/// itself; other formats follow their EXIF orientation.
pub fn decode_oriented(source: &Path) -> Result<image::DynamicImage, ResizeError> {
    if crate::jxl::is_jxl_path(source) {
        return crate::jxl::decode_path(source).map_err(ResizeError::Decode);
    }
    let mut reader = image::ImageReader::open(source)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| ResizeError::Decode(error.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| ResizeError::Decode(error.to_string()))?;
    Ok(apply_exif_orientation(image, source))
}

fn copy_candidate(config: &CopyTarget, label: &str, suffix: u32) -> PathBuf {
    let stem = config
        .source
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("image");
    let base = format!(
        "{stem}-{label}-{}x{}",
        config.target_width, config.target_height
    );
    let filename = if suffix == 0 {
        format!("{base}.webp")
    } else {
        format!("{base}-{suffix}.webp")
    };
    config.destination_dir.join(filename)
}
fn reserve_copy_paths(config: &CopyTarget, label: &str) -> Result<(PathBuf, PathBuf), ResizeError> {
    reserve_output(|suffix| copy_candidate(config, label, suffix))
}

/// Claims the first free name `candidate(0)`, `candidate(1)`, … by creating
/// it exclusively, plus a hidden partial file to write into. The caller
/// renames the partial over the claimed name when done, and removes both on
/// failure. An existing file is never opened for writing.
pub(crate) fn reserve_output(
    candidate: impl Fn(u32) -> PathBuf,
) -> Result<(PathBuf, PathBuf), ResizeError> {
    for suffix in 0..10_000 {
        let path = candidate(suffix);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => {
                let partial = partial_path(&path);
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&partial)
                {
                    Ok(_) => return Ok((path, partial)),
                    Err(error) => {
                        let _ = std::fs::remove_file(&path);
                        if error.kind() != std::io::ErrorKind::AlreadyExists {
                            return Err(ResizeError::Io(error.to_string()));
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(ResizeError::Io(e.to_string())),
        }
    }
    Err(ResizeError::Io("No unused output name is available".into()))
}

pub(crate) fn partial_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("image");
    let extension = output
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("tmp");
    output.with_file_name(format!(
        ".{stem}.partial-{}.{extension}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(1);

    fn test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sharpr-resize-{name}-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn source_image(dir: &Path, dimensions: (u32, u32)) -> PathBuf {
        let source = dir.join("source.png");
        image::DynamicImage::new_rgb8(dimensions.0, dimensions.1)
            .save(&source)
            .unwrap();
        source
    }

    #[test]
    fn png_exif_orientation_controls_dimensions_and_crop_pixels() {
        use image::ImageEncoder;
        let dir = test_dir("png-exif");
        let source = dir.join("oriented.png");
        let mut image = image::RgbImage::new(32, 18);
        for (x, y, p) in image.enumerate_pixels_mut() {
            *p = image::Rgb([x as u8, y as u8, 100]);
        }
        let tiff = vec![
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut encoder =
            image::codecs::png::PngEncoder::new(std::fs::File::create(&source).unwrap());
        encoder.set_exif_metadata(tiff).unwrap();
        encoder
            .write_image(image.as_raw(), 32, 18, image::ExtendedColorType::Rgb8)
            .unwrap();
        let original = std::fs::read(&source).unwrap();
        assert_eq!(oriented_source_dimensions(&source).unwrap(), (18, 32));
        let result = crop_copy(&CropConfig {
            source: source.clone(),
            destination_dir: dir.clone(),
            rectangle: CropRect {
                x: 0,
                y: 0,
                width: 16,
                height: 9,
            },
        })
        .unwrap();
        let pixels = image::open(result.output).unwrap().to_rgb8();
        assert_eq!(pixels.get_pixel(0, 0).0, [0, 17, 100]);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn crop(dir: &Path, rectangle: CropRect) -> CropConfig {
        CropConfig {
            source: source_image(dir, (12, 8)),
            destination_dir: dir.join("out"),
            rectangle,
        }
    }

    const SIX_BY_FOUR: CropRect = CropRect {
        x: 0,
        y: 0,
        width: 6,
        height: 4,
    };

    #[test]
    fn preexisting_partial_image_is_never_overwritten() {
        let dir = test_dir("partial-collision");
        let config = crop(&dir, SIX_BY_FOUR);
        std::fs::create_dir_all(&config.destination_dir).unwrap();
        let target = CopyTarget {
            source: config.source.clone(),
            destination_dir: config.destination_dir.clone(),
            target_width: 6,
            target_height: 4,
        };
        let output = copy_candidate(&target, "cropped", 0);
        let partial = partial_path(&output);
        std::fs::write(&partial, b"existing image bytes").unwrap();
        let result = crop_copy(&config).unwrap();
        assert!(result.output.ends_with("source-cropped-6x4-1.webp"));
        assert_eq!(std::fs::read(&partial).unwrap(), b"existing image bytes");
        assert!(!output.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_webp_fails_without_touching_source_or_creating_outputs() {
        let dir = test_dir("oversized-webp");
        let source = source_image(&dir, (WEBP_MAX_DIMENSION + 1, 1));
        let before = std::fs::read(&source).unwrap();
        let config = CropConfig {
            source: source.clone(),
            destination_dir: dir.join("out"),
            rectangle: CropRect {
                x: 0,
                y: 0,
                width: WEBP_MAX_DIMENSION + 1,
                height: 1,
            },
        };
        assert!(matches!(crop_copy(&config), Err(ResizeError::Encode(_))));
        assert_eq!(std::fs::read(&source).unwrap(), before);
        assert!(!config.destination_dir.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_copy_reservations_never_overwrite_each_other() {
        let dir = test_dir("concurrent");
        let config = crop(&dir, SIX_BY_FOUR);
        std::fs::create_dir_all(&config.destination_dir).unwrap();
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let config = config.clone();
                std::thread::spawn(move || crop_copy(&config).unwrap().output)
            })
            .collect();
        let paths: std::collections::HashSet<_> =
            workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(paths.len(), 4);
        for path in paths {
            assert_eq!(image::image_dimensions(path).unwrap(), (6, 4));
        }
    }

    #[test]
    fn oriented_dimensions_swap_for_quarter_turns() {
        assert_eq!(oriented_dimensions((12, 8), 1), (12, 8));
        assert_eq!(oriented_dimensions((12, 8), 6), (8, 12));
        assert_eq!(oriented_dimensions((12, 8), 8), (8, 12));
    }

    #[test]
    fn unique_names_do_not_overwrite() {
        let dir = test_dir("unique");
        let config = crop(&dir, SIX_BY_FOUR);
        let first = crop_copy(&config).unwrap();
        let second = crop_copy(&config).unwrap();
        assert_ne!(first.output, second.output);
        assert!(second.output.ends_with("source-cropped-6x4-1.webp"));
    }

    #[test]
    fn removes_partial_output_when_encoding_fails() {
        let dir = test_dir("partial");
        let config = crop(&dir, SIX_BY_FOUR);
        let target = CopyTarget {
            source: config.source.clone(),
            destination_dir: config.destination_dir.clone(),
            target_width: 6,
            target_height: 4,
        };
        let output = copy_candidate(&target, "cropped", 0);
        let result = copy_with_encoder(&target, Some(SIX_BY_FOUR), "cropped", |_, path| {
            std::fs::write(path, b"partial").unwrap();
            Err(ResizeError::Encode("forced failure".into()))
        });
        assert!(result.is_err());
        assert!(!partial_path(&output).exists());
        assert!(!output.exists(), "the reserved name is released");
    }
}
