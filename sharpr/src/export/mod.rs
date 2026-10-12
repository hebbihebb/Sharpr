use std::path::{Path, PathBuf};
use std::time::Instant;

use image::imageops::FilterType;

use crate::metadata::orientation::apply_exif_orientation;

pub const WEBP_MAX_DIMENSION: u32 = 16_383;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeFormat {
    Jpeg,
    Png,
    Webp,
    Jxl,
}

impl ResizeFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Jxl => "jxl",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResizeConfig {
    pub source: PathBuf,
    pub destination_dir: PathBuf,
    pub target_width: u32,
    pub target_height: u32,
    pub format: ResizeFormat,
    pub quality: u8,
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

pub fn crop_copy(config: &CropConfig) -> Result<ResizeResult, ResizeError> {
    let resize = ResizeConfig {
        source: config.source.clone(),
        destination_dir: config.destination_dir.clone(),
        target_width: config.rectangle.width,
        target_height: config.rectangle.height,
        format: ResizeFormat::Png,
        quality: 90,
    };
    copy_with_encoder(&resize, Some(config.rectangle), "cropped", encode_image)
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

pub fn default_resize_output_dir() -> PathBuf {
    dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Pictures")))
        .unwrap_or_else(|| PathBuf::from("./Pictures"))
        .join("Resized")
}

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

pub fn height_for_locked_width(source: (u32, u32), width: u32) -> u32 {
    scale_dimension(source.1, source.0, width)
}

pub fn width_for_locked_height(source: (u32, u32), height: u32) -> u32 {
    scale_dimension(source.0, source.1, height)
}

fn scale_dimension(other: u32, changed_source: u32, changed_target: u32) -> u32 {
    if changed_source == 0 {
        return 1;
    }
    ((other as f64 * changed_target as f64 / changed_source as f64).round() as u32).max(1)
}

pub fn resize_copy(config: &ResizeConfig) -> Result<ResizeResult, ResizeError> {
    resize_copy_with_encoder(config, |image, path, format, quality| {
        encode_image(image, path, format, quality)
    })
}

fn resize_copy_with_encoder<F>(
    config: &ResizeConfig,
    encoder: F,
) -> Result<ResizeResult, ResizeError>
where
    F: FnOnce(&image::DynamicImage, &Path, ResizeFormat, u8) -> Result<(), ResizeError>,
{
    copy_with_encoder(config, None, "resized", encoder)
}

fn copy_with_encoder<F>(
    config: &ResizeConfig,
    crop: Option<CropRect>,
    label: &str,
    encoder: F,
) -> Result<ResizeResult, ResizeError>
where
    F: FnOnce(&image::DynamicImage, &Path, ResizeFormat, u8) -> Result<(), ResizeError>,
{
    if config.target_width == 0 || config.target_height == 0 {
        return Err(ResizeError::InvalidDimensions);
    }
    if config.format == ResizeFormat::Webp
        && (config.target_width > WEBP_MAX_DIMENSION || config.target_height > WEBP_MAX_DIMENSION)
    {
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
            "format": config.format.extension(),
        }),
    );

    std::fs::create_dir_all(&config.destination_dir).map_err(|error| {
        ResizeError::Io(format!(
            "create {}: {error}",
            config.destination_dir.display()
        ))
    })?;

    let source = decode_source(&config.source)?;
    let source = apply_exif_orientation(source, &config.source);
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

    let result = encoder(
        &resized,
        &partial,
        config.format,
        config.quality.clamp(1, 100),
    )
    .and_then(|()| {
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

fn decode_source(source: &Path) -> Result<image::DynamicImage, ResizeError> {
    if crate::jxl::is_jxl_path(source) {
        return crate::jxl::decode_path(source).map_err(ResizeError::Decode);
    }
    image::open(source).map_err(|error| ResizeError::Decode(error.to_string()))
}

#[cfg(test)]
fn unique_resize_path(config: &ResizeConfig) -> PathBuf {
    copy_candidate(config, "resized", 0)
}

fn copy_candidate(config: &ResizeConfig, label: &str, suffix: u32) -> PathBuf {
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
        format!("{base}.{}", config.format.extension())
    } else {
        format!("{base}-{suffix}.{}", config.format.extension())
    };
    config.destination_dir.join(filename)
}
fn reserve_copy_paths(
    config: &ResizeConfig,
    label: &str,
) -> Result<(PathBuf, PathBuf), ResizeError> {
    for suffix in 0..10_000 {
        let path = copy_candidate(config, label, suffix);
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

fn partial_path(output: &Path) -> PathBuf {
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

fn encode_image(
    image: &image::DynamicImage,
    output: &Path,
    format: ResizeFormat,
    quality: u8,
) -> Result<(), ResizeError> {
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    use std::io::{BufWriter, Write};

    match format {
        ResizeFormat::Jpeg => {
            let file = std::fs::File::create(output)
                .map_err(|error| ResizeError::Io(error.to_string()))?;
            let rgb = image.to_rgb8();
            let mut writer = BufWriter::new(file);
            JpegEncoder::new_with_quality(&mut writer, quality)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|error| ResizeError::Encode(error.to_string()))?;
            writer
                .flush()
                .map_err(|error| ResizeError::Io(error.to_string()))
        }
        ResizeFormat::Png => {
            let file = std::fs::File::create(output)
                .map_err(|error| ResizeError::Io(error.to_string()))?;
            let rgba = image.to_rgba8();
            let mut writer = BufWriter::new(file);
            PngEncoder::new_with_quality(
                &mut writer,
                CompressionType::Default,
                PngFilter::Adaptive,
            )
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                ExtendedColorType::Rgba8,
            )
            .map_err(|error| ResizeError::Encode(error.to_string()))?;
            writer
                .flush()
                .map_err(|error| ResizeError::Io(error.to_string()))
        }
        ResizeFormat::Webp => {
            let file = std::fs::File::create(output)
                .map_err(|error| ResizeError::Io(error.to_string()))?;
            let mut writer = BufWriter::new(file);
            let rgba = image.to_rgba8();
            let encoded = webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                .encode_simple(false, quality as f32)
                .map_err(|error| ResizeError::Encode(format!("WebP encoding failed: {error:?}")))?;
            writer
                .write_all(encoded.as_ref())
                .map_err(|error| ResizeError::Io(error.to_string()))?;
            writer
                .flush()
                .map_err(|error| ResizeError::Io(error.to_string()))
        }
        ResizeFormat::Jxl => {
            crate::jxl::encode_path(image, output, quality, false, 7).map_err(ResizeError::Encode)
        }
    }
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

    fn config(dir: &Path, format: ResizeFormat) -> ResizeConfig {
        ResizeConfig {
            source: source_image(dir, (12, 8)),
            destination_dir: dir.join("out"),
            target_width: 6,
            target_height: 4,
            format,
            quality: 85,
        }
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

    #[test]
    fn preexisting_partial_image_is_never_overwritten() {
        let dir = test_dir("partial-collision");
        let config = config(&dir, ResizeFormat::Png);
        std::fs::create_dir_all(&config.destination_dir).unwrap();
        let output = copy_candidate(&config, "resized", 0);
        let partial = partial_path(&output);
        std::fs::write(&partial, b"existing image bytes").unwrap();
        let result = resize_copy(&config).unwrap();
        assert!(result.output.ends_with("source-resized-6x4-1.png"));
        assert_eq!(std::fs::read(&partial).unwrap(), b"existing image bytes");
        assert!(!output.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_webp_fails_without_touching_source_or_creating_outputs() {
        let dir = test_dir("oversized-webp");
        let source = source_image(&dir, (WEBP_MAX_DIMENSION + 1, 1));
        let before = std::fs::read(&source).unwrap();
        let config = ResizeConfig {
            source: source.clone(),
            destination_dir: dir.join("out"),
            target_width: WEBP_MAX_DIMENSION + 1,
            target_height: 1,
            format: ResizeFormat::Webp,
            quality: 90,
        };
        assert!(matches!(resize_copy(&config), Err(ResizeError::Encode(_))));
        assert_eq!(std::fs::read(&source).unwrap(), before);
        assert!(!config.destination_dir.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn crop_preserves_original_and_uses_source_pixels() {
        let dir = test_dir("crop");
        let mut pixels = image::RgbaImage::new(32, 18);
        for (x, y, p) in pixels.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8, y as u8, 100, 255]);
        }
        let source = dir.join("source.png");
        pixels.save(&source).unwrap();
        let original = std::fs::read(&source).unwrap();
        let rectangle = CropRect {
            x: 8,
            y: 4,
            width: 16,
            height: 9,
        };
        let config = CropConfig {
            source: source.clone(),
            destination_dir: dir.clone(),
            rectangle,
        };
        let first = crop_copy(&config).unwrap();
        let image = image::open(&first.output).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (16, 9));
        assert_eq!(image.get_pixel(0, 0), pixels.get_pixel(8, 4));
        assert_ne!(crop_copy(&config).unwrap().output, first.output);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert!(!CropRect {
            x: u32::MAX,
            ..rectangle
        }
        .fits((32, 18)));
        assert!(crop_copy(&CropConfig {
            rectangle: CropRect {
                width: 40,
                ..rectangle
            },
            ..config
        })
        .is_err());
    }
    #[test]
    fn crop_coordinates_follow_exif_display_orientation() {
        let dir = test_dir("crop-exif");
        let source = source_image(&dir, (32, 18));
        let jpeg = dir.join("oriented.jpg");
        image::open(&source).unwrap().save(&jpeg).unwrap();
        let bytes = std::fs::read(&jpeg).unwrap();
        let mut exif = b"Exif\0\0II".to_vec();
        exif.extend_from_slice(&42u16.to_le_bytes());
        exif.extend_from_slice(&8u32.to_le_bytes());
        exif.extend_from_slice(&1u16.to_le_bytes());
        exif.extend_from_slice(&0x0112u16.to_le_bytes());
        exif.extend_from_slice(&3u16.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&6u16.to_le_bytes());
        exif.extend_from_slice(&0u16.to_le_bytes());
        exif.extend_from_slice(&0u32.to_le_bytes());
        let mut oriented = bytes[..2].to_vec();
        oriented.extend_from_slice(&[0xff, 0xe1]);
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&bytes[2..]);
        std::fs::write(&jpeg, &oriented).unwrap();
        assert_eq!(oriented_source_dimensions(&jpeg).unwrap(), (18, 32));
        let result = crop_copy(&CropConfig {
            source: jpeg.clone(),
            destination_dir: dir.clone(),
            rectangle: CropRect {
                x: 1,
                y: 20,
                width: 16,
                height: 9,
            },
        })
        .unwrap();
        assert_eq!(image::image_dimensions(result.output).unwrap(), (16, 9));
        assert_eq!(std::fs::read(jpeg).unwrap(), oriented);
    }

    #[test]
    fn unchanged_size_webp_copy_preserves_dimensions_and_source() {
        let dir = test_dir("webp-copy");
        let config = ResizeConfig {
            target_width: 12,
            target_height: 8,
            ..config(&dir, ResizeFormat::Webp)
        };
        let source = std::fs::read(&config.source).unwrap();
        let copy = resize_copy(&config).unwrap();
        assert_eq!(image::image_dimensions(copy.output).unwrap(), (12, 8));
        assert_eq!(std::fs::read(&config.source).unwrap(), source);
    }
    #[test]
    fn concurrent_copy_reservations_never_overwrite_each_other() {
        let dir = test_dir("concurrent");
        let config = config(&dir, ResizeFormat::Png);
        std::fs::create_dir_all(&config.destination_dir).unwrap();
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let config = config.clone();
                std::thread::spawn(move || resize_copy(&config).unwrap().output)
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
    fn locked_dimensions_support_reduction_and_enlargement() {
        assert_eq!(height_for_locked_width((4000, 3000), 2000), 1500);
        assert_eq!(height_for_locked_width((4000, 3000), 8000), 6000);
        assert_eq!(width_for_locked_height((4000, 3000), 1500), 2000);
    }

    #[test]
    fn unlocked_dimensions_remain_independent() {
        let config = ResizeConfig {
            target_width: 7,
            target_height: 3,
            ..config(&test_dir("unlocked"), ResizeFormat::Png)
        };
        let result = resize_copy(&config).unwrap();
        assert_eq!(image::image_dimensions(result.output).unwrap(), (7, 3));
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
        let config = config(&dir, ResizeFormat::Png);
        let first = resize_copy(&config).unwrap();
        let second = resize_copy(&config).unwrap();
        assert_ne!(first.output, second.output);
        assert!(second.output.ends_with("source-resized-6x4-1.png"));
    }

    #[test]
    fn writes_all_supported_formats() {
        for format in [
            ResizeFormat::Jpeg,
            ResizeFormat::Png,
            ResizeFormat::Webp,
            ResizeFormat::Jxl,
        ] {
            let dir = test_dir(format.extension());
            let result = resize_copy(&config(&dir, format)).unwrap();
            assert!(result.output.exists());
            assert_eq!(
                result.output.extension().and_then(|ext| ext.to_str()),
                Some(format.extension())
            );
        }
    }

    #[test]
    fn removes_partial_output_when_encoding_fails() {
        let dir = test_dir("partial");
        let config = config(&dir, ResizeFormat::Png);
        let expected = partial_path(&unique_resize_path(&config));
        let result = resize_copy_with_encoder(&config, |_, path, _, _| {
            std::fs::write(path, b"partial").unwrap();
            Err(ResizeError::Encode("forced failure".into()))
        });
        assert!(result.is_err());
        assert!(!expected.exists());
    }
}
