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
    // libjxl already applies the image's orientation.
    if crate::jxl::is_jxl_path(path) {
        return crate::jxl::decode_path(path).map_err(error);
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

#[cfg(test)]
mod tests {
    use super::*;

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
