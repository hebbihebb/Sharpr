use std::path::{Path, PathBuf};
use std::time::Instant;

use image::imageops::FilterType;

use crate::metadata::orientation::{apply_exif_orientation, exif_orientation};

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
    let dimensions = if crate::jxl::is_jxl_path(source) {
        let image = crate::jxl::decode_path(source).map_err(ResizeError::Decode)?;
        (image.width(), image.height())
    } else {
        image::image_dimensions(source).map_err(|error| ResizeError::Decode(error.to_string()))?
    };
    Ok(oriented_dimensions(dimensions, exif_orientation(source)))
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
    if config.target_width == 0 || config.target_height == 0 {
        return Err(ResizeError::InvalidDimensions);
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
    let resized = source.resize_exact(
        config.target_width,
        config.target_height,
        FilterType::Lanczos3,
    );
    let output = unique_resize_path(config);
    let partial = partial_path(&output);

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

fn unique_resize_path(config: &ResizeConfig) -> PathBuf {
    let stem = config
        .source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("image");
    let base = format!(
        "{stem}-resized-{}x{}",
        config.target_width, config.target_height
    );
    let extension = config.format.extension();
    for suffix in 0..10_000 {
        let filename = if suffix == 0 {
            format!("{base}.{extension}")
        } else {
            format!("{base}-{suffix}.{extension}")
        };
        let candidate = config.destination_dir.join(filename);
        if !candidate.exists() {
            return candidate;
        }
    }
    config
        .destination_dir
        .join(format!("{base}-{}.{}", std::process::id(), extension))
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
            JpegEncoder::new_with_quality(BufWriter::new(file), quality)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|error| ResizeError::Encode(error.to_string()))
        }
        ResizeFormat::Png => {
            let file = std::fs::File::create(output)
                .map_err(|error| ResizeError::Io(error.to_string()))?;
            let rgba = image.to_rgba8();
            PngEncoder::new_with_quality(
                BufWriter::new(file),
                CompressionType::Default,
                PngFilter::Adaptive,
            )
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                ExtendedColorType::Rgba8,
            )
            .map_err(|error| ResizeError::Encode(error.to_string()))
        }
        ResizeFormat::Webp => {
            let file = std::fs::File::create(output)
                .map_err(|error| ResizeError::Io(error.to_string()))?;
            let mut writer = BufWriter::new(file);
            let rgba = image.to_rgba8();
            let encoded = webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                .encode(quality as f32);
            writer
                .write_all(encoded.as_ref())
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
