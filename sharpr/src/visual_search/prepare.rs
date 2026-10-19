//! Turning an image file into what the model sees.
//!
//! The same steps as the Phase 3 benchmark, whose results the user judged:
//! decode, turn upright, fit within 768×768 without enlarging, drop alpha,
//! and encode as PNG. Changing any of this needs a new
//! `PREPARATION_VERSION`.

use std::path::Path;

use image::DynamicImage;

/// The longest edge the model is given.
pub const MAX_EDGE: u32 = 768;

/// The most memory one decode may take.
const MAX_DECODE_BYTES: u64 = 3 << 30;

/// PNG bytes of `path` prepared for the model.
pub fn prepare_png(path: &Path) -> Result<Vec<u8>, String> {
    let image = decode_upright(path)?;
    let (width, height) = fit_within(image.width(), image.height(), MAX_EDGE);
    let image = if (width, height) == (image.width(), image.height()) {
        image
    } else {
        image.resize_exact(width, height, image::imageops::FilterType::CatmullRom)
    };
    let rgb = image.into_rgb8();
    let mut png = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|e| format!("Could not prepare {}: {e}", path.display()))?;
    Ok(png)
}

/// AVIF and HEIC, which decode through glycin's HEIF loader (Work Order
/// item 22): the format's name, or `None` for any other file.
pub(crate) fn heif_format(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "avif" => Some("AVIF"),
        "heic" | "heif" => Some("HEIC"),
        _ => None,
    }
}

/// Part of every error for an AVIF or HEIC image when glycin has no loader
/// for it; such an image counts as unsupported, not unreadable, and is
/// tried again on the next pass.
pub(crate) const NO_LOADER: &str = "needs glycin's HEIF loader";

/// Whether `error` says the image's format can't be decoded here, rather
/// than that the file is unreadable. Older builds wrote "can't be indexed
/// yet" for every AVIF and HEIC image.
pub(crate) fn is_unsupported_error(error: &str) -> bool {
    error.contains(NO_LOADER) || error.contains("images can't be indexed yet")
}

/// The size that fits `width`×`height` within `edge`×`edge`, keeping the
/// aspect ratio and never enlarging.
pub fn fit_within(width: u32, height: u32, edge: u32) -> (u32, u32) {
    if width <= edge && height <= edge {
        return (width.max(1), height.max(1));
    }
    if width >= height {
        let h = (u64::from(height) * u64::from(edge) + u64::from(width) / 2) / u64::from(width);
        (edge, (h as u32).max(1))
    } else {
        let w = (u64::from(width) * u64::from(edge) + u64::from(height) / 2) / u64::from(height);
        ((w as u32).max(1), edge)
    }
}

fn decode_upright(path: &Path) -> Result<DynamicImage, String> {
    let error = |e: String| format!("Could not read {}: {e}", path.display());
    // libjxl already applies the image's orientation. It decodes the
    // whole image, outside the limits below, so check its size first.
    if crate::jxl::is_jxl_path(path) {
        crate::jxl::check_decode_budget(path).map_err(error)?;
        return crate::jxl::decode_path(path).map_err(error);
    }
    if let Some(format) = heif_format(path) {
        return decode_with_glycin(path, format).map_err(error);
    }
    let image = decode_jpeg_reduced(path)
        .map(Ok)
        .unwrap_or_else(|| {
            let mut reader = image::ImageReader::open(path)
                .and_then(|reader| reader.with_guessed_format())
                .map_err(|e| e.to_string())?;
            // The library has images of 270 megapixels and more, past the
            // decoder's default 512 MB limit. One image is decoded at a time.
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(MAX_DECODE_BYTES);
            reader.limits(limits);
            reader.decode().map_err(|e| e.to_string())
        })
        .map_err(error)?;
    Ok(crate::metadata::orientation::apply_exif_orientation(
        image, path,
    ))
}

/// Decodes an AVIF or HEIC image with glycin, the loader the viewer uses. It
/// decodes in a sandboxed process, turns the image upright, and its size is
/// checked against `MAX_DECODE_BYTES` before the pixels are asked for. Runs
/// on the calling worker thread (indexing or thumbnails), blocking it until
/// the image is decoded.
pub(crate) fn decode_with_glycin(path: &Path, format: &str) -> Result<DynamicImage, String> {
    use gtk4::{gio, glib};
    let file = gio::File::for_path(path);
    glib::MainContext::new().block_on(async move {
        let mut loader = glycin::Loader::new(file);
        loader.accepted_memory_formats(
            glycin::MemoryFormatSelection::R8g8b8 | glycin::MemoryFormatSelection::R8g8b8a8,
        );
        let image = loader.load().await.map_err(|e| glycin_error(&e, format))?;
        let details = image.details();
        let (width, height) = (details.width(), details.height());
        if !crate::jxl::fits_decode_budget(width, height) {
            return Err(format!(
                "{width}×{height} is too large to open (over {} GB to decode)",
                MAX_DECODE_BYTES >> 30
            ));
        }
        let frame = image.next_frame().await.map_err(|e| e.to_string())?;
        frame_to_image(&frame)
    })
}

/// The upright size of an AVIF or HEIC image, which glycin reads without
/// decoding the pixels. Blocks the calling worker thread.
pub(crate) fn heif_dimensions(path: &Path, format: &str) -> Result<(u32, u32), String> {
    use gtk4::{gio, glib};
    let file = gio::File::for_path(path);
    glib::MainContext::new().block_on(async move {
        let image = glycin::Loader::new(file)
            .load()
            .await
            .map_err(|e| glycin_error(&e, format))?;
        let details = image.details();
        let (width, height) = (details.width(), details.height());
        Ok(
            match image.transformation_orientation().rotate().degrees() {
                90 | 270 => (height, width),
                _ => (width, height),
            },
        )
    })
}

/// A glycin load error as one line, marking a missing loader.
fn glycin_error(error: &impl std::fmt::Display, format: &str) -> String {
    // glycin appends its whole configuration; the first line says it.
    let e = error.to_string();
    let e = e.lines().next().unwrap_or_default().to_string();
    if e.starts_with("Unknown image format") || e.starts_with("Could not spawn") {
        format!("{format} {NO_LOADER} (glycin-loaders): {e}")
    } else {
        e
    }
}

/// `image` encoded as `mime` (AVIF or HEIC) by glycin at its highest
/// quality, as 8-bit RGB, or RGBA when it has transparency.
pub(crate) fn encode_heif(mime: &'static str, image: &DynamicImage) -> Result<Vec<u8>, String> {
    let (width, height) = (image.width(), image.height());
    let (format, pixels) = if image.color().has_alpha() {
        (glycin::MemoryFormat::R8g8b8a8, image.to_rgba8().into_raw())
    } else {
        (glycin::MemoryFormat::R8g8b8, image.to_rgb8().into_raw())
    };
    gtk4::glib::MainContext::new().block_on(async move {
        let mut creator = glycin::Creator::new(mime.into())
            .await
            .map_err(|e| glycin_error(&e, mime))?;
        creator
            .add_frame(width, height, format, pixels)
            .map_err(|e| e.to_string())?;
        creator
            .set_encoding_quality(100)
            .map_err(|e| e.to_string())?;
        let encoded = creator.create().await.map_err(|e| e.to_string())?;
        encoded.data_full().map_err(|e| e.to_string())
    })
}

/// A glycin frame of 8-bit RGB or RGBA as an `image` image.
fn frame_to_image(frame: &glycin::Frame) -> Result<DynamicImage, String> {
    let (width, height, stride) = (frame.width(), frame.height(), frame.stride() as usize);
    let channels = match frame.memory_format() {
        glycin::MemoryFormat::R8g8b8 => 3,
        glycin::MemoryFormat::R8g8b8a8 => 4,
        other => return Err(format!("unexpected pixel format {other:?}")),
    };
    let row = width as usize * channels;
    let bytes = frame.buf_slice();
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let start = y * stride;
        pixels.extend_from_slice(
            bytes
                .get(start..start + row)
                .ok_or("the decoded image is shorter than its size")?,
        );
    }
    let image = if channels == 3 {
        image::RgbImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8)
    } else {
        image::RgbaImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgba8)
    };
    image.ok_or_else(|| "the decoded image does not match its size".to_string())
}

/// A JPEG decoded at the smallest libjpeg-turbo scale that still leaves at
/// least twice the target size, which is what the benchmark's Pillow
/// `thumbnail` did before its final resize.
fn decode_jpeg_reduced(path: &Path) -> Option<DynamicImage> {
    let is_jpeg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
    if !is_jpeg {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    let mut decompressor = turbojpeg::Decompressor::new().ok()?;
    let header = decompressor.read_header(&data).ok()?;
    if header.is_lossless {
        return None;
    }
    let (tw, th) = fit_within(header.width as u32, header.height as u32, MAX_EDGE);
    let factor = turbojpeg::Decompressor::supported_scaling_factors()
        .into_iter()
        .filter(|f| {
            f.scale(header.width) >= 2 * tw as usize && f.scale(header.height) >= 2 * th as usize
        })
        .min_by_key(|f| f.scale(header.width))
        .unwrap_or(turbojpeg::ScalingFactor::ONE);
    decompressor.set_scaling_factor(factor).ok()?;
    let scaled = header.scaled(factor);
    let pitch = scaled.width * turbojpeg::PixelFormat::RGB.size();
    let mut image = turbojpeg::Image {
        pixels: vec![0; pitch * scaled.height],
        width: scaled.width,
        pitch,
        height: scaled.height,
        format: turbojpeg::PixelFormat::RGB,
    };
    decompressor.decompress(&data, image.as_deref_mut()).ok()?;
    image::RgbImage::from_raw(scaled.width as u32, scaled.height as u32, image.pixels)
        .map(DynamicImage::ImageRgb8)
}

/// Writes a `width`×`height` image of one colour in `mime`, with glycin.
#[cfg(test)]
pub(crate) fn encode_with_glycin(
    mime: &'static str,
    path: &Path,
    (width, height): (u32, u32),
    rgb: [u8; 3],
) {
    let pixels: Vec<u8> = rgb.repeat((width * height) as usize);
    let bytes = gtk4::glib::MainContext::new().block_on(async move {
        let mut creator = glycin::Creator::new(mime.into()).await.unwrap();
        creator
            .add_frame(width, height, glycin::MemoryFormat::R8g8b8, pixels)
            .unwrap();
        creator.create().await.unwrap().data_full().unwrap()
    });
    std::fs::write(path, bytes).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_images_are_turned_away_before_decoding() {
        assert!(crate::jxl::fits_decode_budget(12_000, 8_000));
        assert!(!crate::jxl::fits_decode_budget(60_000, 60_000));
        assert_eq!(heif_format(Path::new("/l/a.AVIF")), Some("AVIF"));
        assert_eq!(heif_format(Path::new("/l/a.heif")), Some("HEIC"));
        assert_eq!(heif_format(Path::new("/l/a.HEIC")), Some("HEIC"));
        assert_eq!(heif_format(Path::new("/l/a.jpg")), None);
        let error = prepare_png(Path::new("/l/missing.avif")).unwrap_err();
        assert!(error.starts_with("Could not read"), "{error}");
    }

    #[test]
    fn a_missing_loader_is_unsupported_not_unreadable() {
        assert!(is_unsupported_error(&format!(
            "AVIF {NO_LOADER} (glycin-loaders)"
        )));
        assert!(is_unsupported_error(
            "HEIC images can't be indexed yet: /l/a.heic"
        ));
        assert!(!is_unsupported_error("Could not read /l/a.avif: truncated"));
    }

    /// Work Order item 22: AVIF and HEIC decode through glycin, upright and
    /// within the edge, like every other format.
    #[test]
    fn avif_and_heic_images_are_prepared() {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-prepare-heif-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (mime, name) in [("image/avif", "green.avif"), ("image/heif", "green.heic")] {
            let path = dir.join(name);
            encode_with_glycin(mime, &path, (1600, 900), [20, 200, 40]);
            let png = prepare_png(&path).unwrap();
            let image = image::load_from_memory(&png).unwrap().into_rgb8();
            assert_eq!(image.dimensions(), (768, 432), "{name}");
            let [r, g, b] = image.get_pixel(384, 216).0;
            assert!(g > 150 && r < 80 && b < 90, "{name}: {r},{g},{b}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn images_fit_within_the_edge_without_enlarging() {
        assert_eq!(fit_within(3840, 2160, 768), (768, 432));
        assert_eq!(fit_within(2160, 3840, 768), (432, 768));
        assert_eq!(fit_within(640, 480, 768), (640, 480));
        assert_eq!(fit_within(10000, 10, 768), (768, 1));
    }

    #[test]
    fn prepared_images_are_upright_rgb_png_within_the_edge() {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-prepare-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // A wide JPEG with a red left half.
        let mut source = image::RgbImage::new(2000, 1000);
        for (x, _, pixel) in source.enumerate_pixels_mut() {
            *pixel = if x < 1000 {
                image::Rgb([220, 20, 20])
            } else {
                image::Rgb([20, 20, 220])
            };
        }
        let jpeg = dir.join("wide.jpg");
        source.save(&jpeg).unwrap();
        let before = std::fs::read(&jpeg).unwrap();

        let png = prepare_png(&jpeg).unwrap();
        let prepared = image::load_from_memory(&png).unwrap();
        assert_eq!((prepared.width(), prepared.height()), (768, 384));
        assert!(matches!(prepared, DynamicImage::ImageRgb8(_)));
        let left = prepared.to_rgb8().get_pixel(10, 192).0;
        assert!(left[0] > 180 && left[2] < 60, "{left:?}");
        assert_eq!(std::fs::read(&jpeg).unwrap(), before, "source untouched");

        // A small PNG with alpha keeps its size and loses the alpha.
        let rgba = image::RgbaImage::from_pixel(50, 80, image::Rgba([1, 2, 3, 0]));
        let small = dir.join("small.png");
        rgba.save(&small).unwrap();
        let prepared = image::load_from_memory(&prepare_png(&small).unwrap()).unwrap();
        assert_eq!((prepared.width(), prepared.height()), (50, 80));
        assert_eq!(prepared.to_rgb8().get_pixel(0, 0).0, [1, 2, 3]);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
