use std::path::{Path, PathBuf};
use std::time::Instant;

use image::imageops::FilterType;

use crate::metadata::orientation::apply_exif_orientation;

pub mod batch;
mod crop_format;

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

/// A corner of a crop selection, dragged to resize it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The smallest crop side the selection can be dragged down to.
const MIN_CROP: u32 = 16;

impl CropRect {
    /// The largest `aspect` rectangle centred in `bounds`, or the whole
    /// image for a free aspect.
    pub fn centered(bounds: (u32, u32), aspect: Option<(u32, u32)>) -> Self {
        match aspect {
            Some((16, 9)) => Self::centered_widescreen(bounds),
            Some((a, b)) => {
                let scale =
                    (f64::from(bounds.0) / f64::from(a)).min(f64::from(bounds.1) / f64::from(b));
                let (w, h) = (
                    ((f64::from(a) * scale) as u32).clamp(1, bounds.0),
                    ((f64::from(b) * scale) as u32).clamp(1, bounds.1),
                );
                Self {
                    x: (bounds.0 - w) / 2,
                    y: (bounds.1 - h) / 2,
                    width: w,
                    height: h,
                }
            }
            None => Self {
                x: 0,
                y: 0,
                width: bounds.0.max(1),
                height: bounds.1.max(1),
            },
        }
    }

    /// The selection for a `frame` given as fractions of the image (x, y,
    /// width, height), such as the viewer's 16:9 guide (D38): the part of
    /// the frame inside the image, or with an `aspect` the largest whole a×b
    /// rectangle centred in that part. Falls back to [`Self::centered`] when
    /// too little of the frame is inside.
    pub fn framed(
        bounds: (u32, u32),
        frame: (f64, f64, f64, f64),
        aspect: Option<(u32, u32)>,
    ) -> Self {
        let (bw, bh) = (f64::from(bounds.0), f64::from(bounds.1));
        let left = (frame.0 * bw).clamp(0.0, bw);
        let top = (frame.1 * bh).clamp(0.0, bh);
        let right = ((frame.0 + frame.2) * bw).clamp(0.0, bw);
        let bottom = ((frame.1 + frame.3) * bh).clamp(0.0, bh);
        let (room_w, room_h) = (right - left, bottom - top);
        let (w, h) = match aspect {
            Some((a, b)) => {
                let units = (room_w / f64::from(a)).min(room_h / f64::from(b)).floor();
                (units * f64::from(a), units * f64::from(b))
            }
            None => (room_w.round(), room_h.round()),
        };
        if w < 1.0 || h < 1.0 {
            return Self::centered(bounds, aspect);
        }
        let x = (left + (room_w - w) / 2.0).round().clamp(0.0, bw - w);
        let y = (top + (room_h - h) / 2.0).round().clamp(0.0, bh - h);
        Self {
            x: x as u32,
            y: y as u32,
            width: w as u32,
            height: h as u32,
        }
    }

    /// Moved by (`dx`, `dy`) image pixels, kept inside `bounds`.
    pub fn moved(self, dx: f64, dy: f64, bounds: (u32, u32)) -> Self {
        let clamp = |start: u32, delta: f64, size: u32, limit: u32| {
            (f64::from(start) + delta)
                .round()
                .clamp(0.0, f64::from(limit.saturating_sub(size))) as u32
        };
        Self {
            x: clamp(self.x, dx, self.width, bounds.0),
            y: clamp(self.y, dy, self.height, bounds.1),
            ..self
        }
    }

    /// Resized by dragging `corner` by (`dx`, `dy`); the opposite corner
    /// stays put. With an `aspect`, the drag along the diagonal decides the
    /// size and the ratio holds. The result stays inside `bounds`.
    pub fn resized(
        self,
        corner: Corner,
        dx: f64,
        dy: f64,
        bounds: (u32, u32),
        aspect: Option<(u32, u32)>,
    ) -> Self {
        let (left, top) = (f64::from(self.x), f64::from(self.y));
        let (right, bottom) = (left + f64::from(self.width), top + f64::from(self.height));
        // The fixed corner, and how far the room extends from it.
        let (anchor_x, anchor_y, grow_x, grow_y) = match corner {
            Corner::TopLeft => (right, bottom, -1.0, -1.0),
            Corner::TopRight => (left, bottom, 1.0, -1.0),
            Corner::BottomLeft => (right, top, -1.0, 1.0),
            Corner::BottomRight => (left, top, 1.0, 1.0),
        };
        let room_x = if grow_x > 0.0 {
            f64::from(bounds.0) - anchor_x
        } else {
            anchor_x
        };
        let room_y = if grow_y > 0.0 {
            f64::from(bounds.1) - anchor_y
        } else {
            anchor_y
        };
        let min = f64::from(MIN_CROP);
        let mut w = (f64::from(self.width) + dx * grow_x).clamp(min.min(room_x), room_x);
        let mut h = (f64::from(self.height) + dy * grow_y).clamp(min.min(room_y), room_y);
        if let Some((a, b)) = aspect {
            let (a, b) = (f64::from(a), f64::from(b));
            // Follow the drag projected onto the selection's diagonal, as the
            // viewer's guide does (D38), so straight drags shrink as well as
            // grow; in whole a×b units so the ratio is exact, within the room.
            let units =
                f64::from(self.width) / a + (dx * grow_x * a + dy * grow_y * b) / (a * a + b * b);
            let most = (room_x / a).floor().min((room_y / b).floor()).max(1.0);
            let least = (min / b).ceil().min(most);
            let units = units.round().clamp(least, most);
            w = units * a;
            h = units * b;
        }
        let (w, h) = (w.round().max(1.0), h.round().max(1.0));
        let x = if grow_x > 0.0 { anchor_x } else { anchor_x - w };
        let y = if grow_y > 0.0 { anchor_y } else { anchor_y - h };
        Self {
            x: x.max(0.0) as u32,
            y: y.max(0.0) as u32,
            width: w as u32,
            height: h as u32,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CropConfig {
    pub source: PathBuf,
    pub destination_dir: PathBuf,
    pub rectangle: CropRect,
}

/// Writes the displayed crop as a new copy in the source format.
pub fn crop_copy(config: &CropConfig) -> Result<ResizeResult, ResizeError> {
    let format = crop_format::CropFormat::detect(&config.source)?;
    let extension = format.extension(&config.source);
    let target = CopyTarget {
        source: config.source.clone(),
        destination_dir: config.destination_dir.clone(),
        target_width: config.rectangle.width,
        target_height: config.rectangle.height,
    };
    copy_with_encoder(
        &target,
        Some(config.rectangle),
        "cropped",
        &extension,
        |image| format.encode(image),
    )
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

/// A new output file on its way: a hidden partial file created exclusively
/// and written only through its own handle, then published under the first
/// free candidate name by a rename that never replaces a file. Dropped
/// unpublished, it removes the partial file, but only if that is still the
/// file it created; nothing else is ever removed or overwritten.
pub(crate) struct Pending {
    partial: PathBuf,
    file: std::fs::File,
    id: (u64, u64),
    candidate: Box<dyn Fn(u32) -> PathBuf>,
    suffix: u32,
    published: bool,
}

fn file_id(metadata: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

/// Renames `from` to `to` unless `to` exists, atomically where the file
/// system allows (`renameat2` with `RENAME_NOREPLACE`, else a hard link).
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |path: &Path| {
        std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    let (from_c, to_c) = (c(from)?, c(to)?);
    // SAFETY: both are valid NUL-terminated paths for the call's duration.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        // Not supported here: a hard link also fails if `to` exists.
        Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP) => {
            std::fs::hard_link(from, to)?;
            std::fs::remove_file(from)
        }
        _ => Err(error),
    }
}

impl Pending {
    /// Claims a partial file beside the first candidate name that is free.
    pub(crate) fn reserve(
        candidate: impl Fn(u32) -> PathBuf + 'static,
    ) -> Result<Self, ResizeError> {
        for suffix in 0..10_000 {
            let path = candidate(suffix);
            if std::fs::symlink_metadata(&path).is_ok() {
                continue;
            }
            let partial = partial_path(&path);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&partial)
            {
                Ok(file) => {
                    let id = file
                        .metadata()
                        .map(|m| file_id(&m))
                        .map_err(|e| ResizeError::Io(e.to_string()))?;
                    return Ok(Self {
                        partial,
                        file,
                        id,
                        candidate: Box::new(candidate),
                        suffix,
                        published: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(ResizeError::Io(e.to_string())),
            }
        }
        Err(ResizeError::Io("No unused output name is available".into()))
    }

    /// Writes `bytes` and makes sure they reached the disk.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), ResizeError> {
        use std::io::Write;
        self.file
            .write_all(bytes)
            .and_then(|()| self.file.sync_all())
            .map_err(|e| ResizeError::Io(format!("writing {}: {e}", self.partial.display())))
    }

    fn still_ours(&self) -> bool {
        std::fs::symlink_metadata(&self.partial).is_ok_and(|m| file_id(&m) == self.id)
    }

    /// Streams an ordinary file copy into this owned partial, without
    /// holding the whole image in memory or opening its final name for writing.
    pub(crate) fn copy_from(&mut self, source: &mut std::fs::File) -> Result<(), ResizeError> {
        std::io::copy(source, &mut self.file)
            .and_then(|_| source.metadata())
            .and_then(|metadata| self.file.set_permissions(metadata.permissions()))
            .and_then(|_| self.file.sync_all())
            .map_err(|e| ResizeError::Io(format!("copying {}: {e}", self.partial.display())))
    }

    /// Moves the written file to the first free name from the claimed one
    /// on, never replacing a file that appeared there meanwhile.
    pub(crate) fn publish(mut self) -> Result<PathBuf, ResizeError> {
        if !self.still_ours() {
            return Err(ResizeError::Io(format!(
                "{} was replaced while writing",
                self.partial.display()
            )));
        }
        for suffix in self.suffix..10_000 {
            let output = (self.candidate)(suffix);
            match rename_no_replace(&self.partial, &output) {
                Ok(()) => {
                    self.published = true;
                    return Ok(output);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(ResizeError::Io(format!(
                        "move {} to {}: {e}",
                        self.partial.display(),
                        output.display()
                    )))
                }
            }
        }
        Err(ResizeError::Io("No unused output name is available".into()))
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.published && self.still_ours() {
            let _ = std::fs::remove_file(&self.partial);
        }
    }
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
    // The `image` crate can't read AVIF or HEIC headers; glycin can.
    if let Some(format) = crate::visual_search::prepare::heif_format(source) {
        return crate::visual_search::prepare::heif_dimensions(source, format)
            .map_err(ResizeError::Decode);
    }
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
    extension: &str,
    encoder: F,
) -> Result<ResizeResult, ResizeError>
where
    F: FnOnce(&image::DynamicImage) -> Result<Vec<u8>, ResizeError>,
{
    if config.target_width == 0 || config.target_height == 0 {
        return Err(ResizeError::InvalidDimensions);
    }
    if extension == "webp"
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
    let target = config.clone();
    let label = label.to_string();
    let extension = extension.to_string();
    let mut pending =
        Pending::reserve(move |suffix| copy_candidate(&target, &label, &extension, suffix))?;
    let result = encoder(&resized).and_then(|bytes| {
        pending.write(&bytes)?;
        pending.publish()
    });
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            crate::bench_error!(
                "resize.copy.fail",
                serde_json::json!({
                    "source": config.source.display().to_string(),
                    "error": error.to_string(),
                }),
            );
            return Err(error);
        }
    };

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
/// and glycin AVIF and HEIC itself; other formats follow their EXIF
/// orientation.
pub fn decode_oriented(source: &Path) -> Result<image::DynamicImage, ResizeError> {
    if crate::jxl::is_jxl_path(source) {
        // libjxl applies the orientation itself, outside the limits below.
        crate::jxl::check_decode_budget(source).map_err(ResizeError::Decode)?;
        return crate::jxl::decode_path(source).map_err(ResizeError::Decode);
    }
    // The `image` crate reads neither AVIF nor HEIC; glycin checks the size
    // against the same budget before decoding (Compare, item 32).
    if let Some(format) = crate::visual_search::prepare::heif_format(source) {
        return crate::visual_search::prepare::decode_with_glycin(source, format)
            .map_err(ResizeError::Decode);
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

fn copy_candidate(config: &CopyTarget, label: &str, extension: &str, suffix: u32) -> PathBuf {
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
        format!("{base}.{extension}")
    } else {
        format!("{base}-{suffix}.{extension}")
    };
    config.destination_dir.join(filename)
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

    #[test]
    fn crop_preserves_source_format_and_original_bytes() {
        let dir = test_dir("source-format");
        for (extension, format) in [
            ("png", image::ImageFormat::Png),
            ("jpeg", image::ImageFormat::Jpeg),
            ("tif", image::ImageFormat::Tiff),
            ("bmp", image::ImageFormat::Bmp),
            ("gif", image::ImageFormat::Gif),
            ("ico", image::ImageFormat::Ico),
            ("webp", image::ImageFormat::WebP),
        ] {
            let source = dir.join(format!("source.{extension}"));
            let pixels = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                32,
                18,
                image::Rgb([40, 90, 150]),
            ));
            if format == image::ImageFormat::Ico {
                image::DynamicImage::ImageRgba8(pixels.to_rgba8())
                    .save(&source)
                    .unwrap();
            } else {
                pixels.save(&source).unwrap();
            }
            let original = std::fs::read(&source).unwrap();
            let config = CropConfig {
                source: source.clone(),
                destination_dir: dir.join("out"),
                rectangle: CropRect {
                    x: 4,
                    y: 2,
                    width: 16,
                    height: 9,
                },
            };
            let first = crop_copy(&config).unwrap();
            assert_eq!(
                image::guess_format(&std::fs::read(&first.output).unwrap()).unwrap(),
                format,
                "crop must keep {extension}"
            );
            assert_eq!(first.output.extension().unwrap(), extension);
            assert_eq!(image::image_dimensions(&first.output).unwrap(), (16, 9));
            let bytes = std::fs::read(&first.output).unwrap();
            let second = crop_copy(&config).unwrap();
            assert_ne!(first.output, second.output);
            assert_eq!(std::fs::read(&first.output).unwrap(), bytes);
            assert_eq!(std::fs::read(&source).unwrap(), original);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn png_and_tiff_crop_preserve_sixteen_bit_pixels_and_alpha() {
        let dir = test_dir("crop-sixteen-bit");
        for extension in ["png", "tiff"] {
            let source = dir.join(format!("gradient.{extension}"));
            let pixels = image::ImageBuffer::from_fn(32, 18, |x, y| {
                image::Rgba([x as u16 * 2000, y as u16 * 3000, 12345, 45678])
            });
            image::DynamicImage::ImageRgba16(pixels.clone())
                .save(&source)
                .unwrap();
            let result = crop_copy(&CropConfig {
                source,
                destination_dir: dir.clone(),
                rectangle: CropRect {
                    x: 3,
                    y: 2,
                    width: 16,
                    height: 9,
                },
            })
            .unwrap();
            let actual = image::open(result.output).unwrap();
            assert_eq!(actual.color(), image::ColorType::Rgba16);
            assert_eq!(
                actual.to_rgba16(),
                image::imageops::crop_imm(&pixels, 3, 2, 16, 9).to_image()
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn jpeg_xl_crop_stays_jpeg_xl_with_exact_pixels() {
        let dir = test_dir("crop-jxl");
        let source = dir.join("source.jxl");
        let pixels = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(32, 18, |x, y| {
            image::Rgba([x as u8 * 7, y as u8 * 11, 90, 170])
        }));
        crate::jxl::encode_path(&pixels, &source, 100, false, 3).unwrap();
        let original = std::fs::read(&source).unwrap();
        let expected = decode_oriented(&source).unwrap().crop_imm(3, 2, 16, 9);
        let result = crop_copy(&CropConfig {
            source: source.clone(),
            destination_dir: dir.clone(),
            rectangle: CropRect {
                x: 3,
                y: 2,
                width: 16,
                height: 9,
            },
        })
        .unwrap();
        assert_eq!(result.output.extension().unwrap(), "jxl");
        assert_eq!(
            decode_oriented(&result.output).unwrap().to_rgba8(),
            expected.to_rgba8()
        );
        assert_eq!(std::fs::read(source).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// AVIF and HEIC crops keep their format (D37, like JPEG at the highest
    /// quality) and their size reads through glycin; the original is untouched.
    #[test]
    fn avif_and_heic_crops_keep_their_format() {
        let dir = test_dir("crop-heif");
        // Red on the left, blue on the right.
        let pixels = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(128, 64, |x, _| {
            if x < 64 {
                image::Rgb([220, 30, 30])
            } else {
                image::Rgb([30, 30, 220])
            }
        }));
        for (mime, extension) in [("image/avif", "avif"), ("image/heif", "heic")] {
            let source = dir.join(format!("source.{extension}"));
            let bytes = crate::visual_search::prepare::encode_heif(mime, &pixels).unwrap();
            std::fs::write(&source, &bytes).unwrap();
            assert_eq!(oriented_source_dimensions(&source).unwrap(), (128, 64));
            let result = crop_copy(&CropConfig {
                source: source.clone(),
                destination_dir: dir.clone(),
                rectangle: CropRect {
                    x: 72,
                    y: 8,
                    width: 48,
                    height: 40,
                },
            })
            .unwrap();
            assert_eq!(result.output.extension().unwrap(), extension);
            assert_eq!(
                oriented_source_dimensions(&result.output).unwrap(),
                (48, 40)
            );
            let cropped = decode_oriented(&result.output).unwrap().to_rgb8();
            let [r, g, b] = cropped.get_pixel(24, 20).0;
            assert!(b > 170 && r < 80 && g < 80, "{extension}: {r},{g},{b}");
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn png_crop_does_not_inherit_webp_size_limits() {
        let dir = test_dir("wide-png-crop");
        let source = source_image(&dir, (WEBP_MAX_DIMENSION + 1, 1));
        let result = crop_copy(&CropConfig {
            source,
            destination_dir: dir.clone(),
            rectangle: CropRect {
                x: 0,
                y: 0,
                width: WEBP_MAX_DIMENSION + 1,
                height: 1,
            },
        })
        .unwrap();
        assert_eq!(
            image::image_dimensions(result.output).unwrap(),
            (WEBP_MAX_DIMENSION + 1, 1)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mislabeled_png_crop_uses_the_real_format() {
        let dir = test_dir("crop-real-format");
        let source = source_image(&dir, (32, 18));
        let mislabeled = source.with_extension("jpg");
        std::fs::rename(&source, &mislabeled).unwrap();
        let result = crop_copy(&CropConfig {
            source: mislabeled,
            destination_dir: dir.clone(),
            rectangle: SIX_BY_FOUR,
        })
        .unwrap();
        assert_eq!(result.output.extension().unwrap(), "png");
        assert_eq!(
            image::guess_format(&std::fs::read(result.output).unwrap()).unwrap(),
            image::ImageFormat::Png
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsupported_crop_format_fails_without_outputs_or_source_changes() {
        let dir = test_dir("crop-unsupported");
        let source = dir.join("source.pnm");
        let bytes = b"P6\n2 1\n255\nabcdef";
        std::fs::write(&source, bytes).unwrap();
        let destination = dir.join("out");
        let result = crop_copy(&CropConfig {
            source: source.clone(),
            destination_dir: destination.clone(),
            rectangle: CropRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        });
        assert!(
            matches!(result, Err(ResizeError::Encode(message)) if message.contains("original format"))
        );
        assert!(!destination.exists());
        assert_eq!(std::fs::read(source).unwrap(), bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn jpeg_xl_crop_retains_sixteen_bit_and_float_samples() {
        let dir = test_dir("crop-jxl-depth");
        let images = [
            image::DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(32, 18, |x, y| {
                image::Rgba([x as u16 * 2000, y as u16 * 3000, 12345, 45678])
            })),
            image::DynamicImage::ImageRgb32F(image::ImageBuffer::from_fn(32, 18, |x, y| {
                image::Rgb([x as f32 / 32.0, y as f32 / 32.0, 0.25])
            })),
        ];
        for (n, pixels) in images.into_iter().enumerate() {
            let source = dir.join(format!("depth-{n}.jxl"));
            std::fs::write(
                &source,
                crop_format::CropFormat::Jxl.encode(&pixels).unwrap(),
            )
            .unwrap();
            assert_eq!(
                decode_oriented(&source).unwrap().as_bytes(),
                pixels.as_bytes(),
                "fixture samples are unchanged"
            );
            let result = crop_copy(&CropConfig {
                source,
                destination_dir: dir.clone(),
                rectangle: CropRect {
                    x: 3,
                    y: 2,
                    width: 16,
                    height: 9,
                },
            })
            .unwrap();
            let actual = decode_oriented(&result.output).unwrap();
            let expected = pixels.crop_imm(3, 2, 16, 9);
            assert_eq!(actual.color(), expected.color());
            assert_eq!(actual.as_bytes(), expected.as_bytes());
        }
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
        let output = copy_candidate(&target, "cropped", "png", 0);
        let partial = partial_path(&output);
        std::fs::write(&partial, b"existing image bytes").unwrap();
        let result = crop_copy(&config).unwrap();
        assert!(result.output.ends_with("source-cropped-6x4-1.png"));
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
        let target = CopyTarget {
            source: config.source.clone(),
            destination_dir: config.destination_dir.clone(),
            target_width: config.rectangle.width,
            target_height: config.rectangle.height,
        };
        assert!(matches!(
            copy_with_encoder(
                &target,
                Some(config.rectangle),
                "cropped",
                "webp",
                |_| unreachable!()
            ),
            Err(ResizeError::Encode(_))
        ));
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

    /// A JPEG stored sideways (EXIF orientation 6, a quarter turn
    /// clockwise): the crop follows what the viewer shows.
    #[test]
    fn cropping_a_rotated_jpeg_takes_the_displayed_pixels() {
        let dir = test_dir("crop-exif-pixels");
        // Stored 32×18: left half red, right half blue. Displayed upright
        // (18×32), red is on top and blue below.
        let stored = image::RgbImage::from_fn(32, 18, |x, _| {
            if x < 16 {
                image::Rgb([220, 20, 20])
            } else {
                image::Rgb([20, 20, 220])
            }
        });
        let plain = dir.join("plain.jpg");
        stored.save(&plain).unwrap();
        let bytes = std::fs::read(&plain).unwrap();
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
        let jpeg = dir.join("rotated.jpg");
        std::fs::write(&jpeg, &oriented).unwrap();

        let crop = |y| {
            let result = crop_copy(&CropConfig {
                source: jpeg.clone(),
                destination_dir: dir.join("out"),
                rectangle: CropRect {
                    x: 2,
                    y,
                    width: 14,
                    height: 12,
                },
            })
            .unwrap();
            let pixels = image::open(result.output).unwrap().to_rgb8();
            pixels.get_pixel(7, 6).0
        };
        let top = crop(2);
        let bottom = crop(18);
        assert!(
            top[0] > 150 && top[2] < 90,
            "top of the displayed image is red: {top:?}"
        );
        assert!(
            bottom[2] > 150 && bottom[0] < 90,
            "bottom is blue: {bottom:?}"
        );
        assert_eq!(std::fs::read(&jpeg).unwrap(), oriented, "source untouched");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn framed_selections_follow_the_guide() {
        let bounds = (3200, 1800);
        // A guide wholly inside the image becomes that 16:9 selection.
        assert_eq!(
            CropRect::framed(bounds, (0.25, 0.25, 0.5, 0.5), Some((16, 9))),
            CropRect {
                x: 800,
                y: 450,
                width: 1600,
                height: 900
            }
        );
        // Overhanging the image's right side: the largest 16:9 inside the
        // overlap, centred on it.
        let r = CropRect::framed(bounds, (0.75, 0.0, 0.5, 1.0), Some((16, 9)));
        assert!(r.fits(bounds));
        assert_eq!((r.width * 9, r.y + r.height / 2), (r.height * 16, 900));
        assert_eq!(r.x + r.width / 2, 2800);
        // A free aspect keeps the overlap as it is.
        assert_eq!(
            CropRect::framed((1000, 1000), (-0.5, 0.1, 1.0, 0.5), None),
            CropRect {
                x: 0,
                y: 100,
                width: 500,
                height: 500
            }
        );
        // Nothing inside: the usual centred selection.
        assert_eq!(
            CropRect::framed(bounds, (2.0, 2.0, 0.5, 0.5), Some((16, 9))),
            CropRect::centered(bounds, Some((16, 9)))
        );
    }

    #[test]
    fn crop_selections_move_and_resize_inside_the_image() {
        let bounds = (1920, 1080);
        let rect = CropRect::centered(bounds, Some((16, 9)));
        assert_eq!(
            rect,
            CropRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(CropRect::centered((1000, 1000), None).width, 1000);
        let small = CropRect {
            x: 100,
            y: 100,
            width: 320,
            height: 180,
        };
        assert_eq!(
            small.moved(-500.0, 20.0, bounds),
            CropRect {
                x: 0,
                y: 120,
                ..small
            }
        );
        assert_eq!(small.moved(5000.0, 0.0, bounds).x, 1920 - 320);

        // Free: the opposite corner stays, each side follows the drag.
        let free = small.resized(Corner::BottomRight, 80.0, 20.0, bounds, None);
        assert_eq!(
            free,
            CropRect {
                x: 100,
                y: 100,
                width: 400,
                height: 200
            }
        );
        let from_top_left = small.resized(Corner::TopLeft, 50.0, 50.0, bounds, None);
        assert_eq!(
            from_top_left,
            CropRect {
                x: 150,
                y: 150,
                width: 270,
                height: 130
            }
        );

        // 16:9 holds; a drag along the diagonal moves the corner with it,
        // and the image edge stops it.
        let wide = small.resized(Corner::BottomRight, 160.0, 90.0, bounds, Some((16, 9)));
        assert_eq!((wide.width, wide.height), (480, 270));
        // A straight drag resizes too, growing or shrinking.
        for (dx, dy, grows) in [(160.0, 0.0, true), (0.0, 90.0, true), (-160.0, 0.0, false)] {
            let r = small.resized(Corner::BottomRight, dx, dy, bounds, Some((16, 9)));
            assert_eq!(r.width * 9, r.height * 16);
            assert_eq!(r.width > 320, grows, "{r:?}");
            assert_ne!(r.width, 320, "{r:?}");
            assert_eq!((r.x, r.y), (100, 100));
        }
        let huge = small.resized(Corner::BottomRight, 9000.0, 9000.0, bounds, Some((16, 9)));
        assert!(huge.fits(bounds), "{huge:?}");
        assert_eq!(huge.width * 9, huge.height * 16);
        let tiny = small.resized(Corner::BottomRight, -9000.0, -9000.0, bounds, None);
        assert_eq!(
            (tiny.width, tiny.height),
            (16, 16),
            "never below the minimum"
        );
    }

    #[test]
    fn aspect_resizing_never_jumps() {
        // Growing one way while shrinking the other changes the size a
        // little for a little movement, never by a leap.
        let start = CropRect {
            x: 100,
            y: 100,
            width: 320,
            height: 180,
        };
        let mut last = start.width;
        for step in 1..=200 {
            let t = f64::from(step);
            let r = start.resized(
                Corner::BottomRight,
                t,
                -t * 0.9,
                (1920, 1080),
                Some((16, 9)),
            );
            assert!(r.width.abs_diff(last) <= 16, "{last} -> {}", r.width);
            last = r.width;
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
        assert!(second.output.ends_with("source-cropped-6x4-1.png"));
    }

    #[test]
    fn publishing_never_replaces_a_file_that_appeared_meanwhile() {
        let dir = test_dir("no-replace");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.clone();
        let name = move |suffix: u32| base.join(format!("out-{suffix}.webp"));
        let mut pending = Pending::reserve(name.clone()).unwrap();
        pending.write(b"ours").unwrap();
        // Another program writes the chosen name before publishing.
        std::fs::write(name(0), b"theirs").unwrap();
        let output = pending.publish().unwrap();
        assert_eq!(output, name(1));
        assert_eq!(std::fs::read(name(0)).unwrap(), b"theirs");
        assert_eq!(std::fs::read(name(1)).unwrap(), b"ours");

        // A partial file replaced from outside is neither published nor
        // removed.
        let mut pending = Pending::reserve(name.clone()).unwrap();
        pending.write(b"ours").unwrap();
        let partial = pending.partial.clone();
        std::fs::remove_file(&partial).unwrap();
        std::fs::write(&partial, b"replacement").unwrap();
        assert!(pending.publish().is_err());
        assert_eq!(std::fs::read(&partial).unwrap(), b"replacement");
        assert!(!name(2).exists());
        std::fs::remove_dir_all(dir).unwrap();
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
        let output = copy_candidate(&target, "cropped", "png", 0);
        let result = copy_with_encoder(&target, Some(SIX_BY_FOUR), "cropped", "png", |_| {
            Err(ResizeError::Encode("forced failure".into()))
        });
        assert!(result.is_err());
        assert!(!partial_path(&output).exists());
        assert!(!output.exists(), "the reserved name is released");
    }
}
