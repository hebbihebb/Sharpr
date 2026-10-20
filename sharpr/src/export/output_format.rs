//! The same explicit format choice for crops and exports (D47).
use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageFormat};

use super::{crop_format::CropFormat, ResizeError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Png,
    Webp,
    Original,
}

impl OutputFormat {
    pub fn key(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Original => "original",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "webp" => Self::Webp,
            "original" => Self::Original,
            _ => Self::Png,
        }
    }

    pub(super) fn resolve(self, source: &Path) -> Result<CropFormat, ResizeError> {
        match self {
            Self::Png => Ok(CropFormat::Raster(ImageFormat::Png)),
            Self::Webp => Ok(CropFormat::Raster(ImageFormat::WebP)),
            Self::Original => CropFormat::detect(source),
        }
    }

    pub(super) fn encode(
        self,
        source: &Path,
        image: &DynamicImage,
        lossless: bool,
        quality: u8,
    ) -> Result<Vec<u8>, ResizeError> {
        let format = self.resolve(source)?;
        match format {
            CropFormat::Raster(ImageFormat::Png) => png_bytes(source, image),
            CropFormat::Raster(ImageFormat::WebP) => {
                // Original WebP, like Crop Copy, always uses lossless encoding.
                super::webp_bytes(image, self != Self::Webp || lossless, quality)
            }
            _ => format.encode(image),
        }
    }
}

fn png_bytes(source: &Path, image: &DynamicImage) -> Result<Vec<u8>, ResizeError> {
    if matches!(
        image.color(),
        image::ColorType::Rgb32F | image::ColorType::Rgba32F
    ) {
        return Err(ResizeError::Encode(
            "PNG cannot preserve floating-point/HDR samples; choose Original or convert to SDR in an external editor".into(),
        ));
    }
    let mut info = png::Info::with_size(image.width(), image.height());
    info.color_type = match image.color() {
        image::ColorType::L8 | image::ColorType::L16 => png::ColorType::Grayscale,
        image::ColorType::La8 | image::ColorType::La16 => png::ColorType::GrayscaleAlpha,
        image::ColorType::Rgb8 | image::ColorType::Rgb16 => png::ColorType::Rgb,
        _ => png::ColorType::Rgba,
    };
    let sixteen = image.color().bits_per_pixel() / u16::from(image.color().channel_count()) == 16;
    info.bit_depth = if sixteen {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    };
    // Standard raster decoders return the source samples without color conversion.
    // Carry their ICC alongside those samples, but not EXIF orientation (pixels
    // are already upright). Special decoders have their own output color handling.
    if !crate::jxl::is_jxl_path(source)
        && crate::visual_search::prepare::heif_format(source).is_none()
    {
        let mut reader = image::ImageReader::open(source)
            .and_then(|r| r.with_guessed_format())
            .map_err(|e| ResizeError::Decode(e.to_string()))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(super::MAX_DECODE_BYTES);
        reader.limits(limits);
        let source_is_png = reader.format() == Some(ImageFormat::Png);
        let mut decoder = reader
            .into_decoder()
            .map_err(|e| ResizeError::Decode(e.to_string()))?;
        if let Some(icc) = decoder
            .icc_profile()
            .map_err(|e| ResizeError::Decode(e.to_string()))?
        {
            let expected = if image.color().channel_count() <= 2 {
                b"GRAY"
            } else {
                b"RGB "
            };
            if icc.get(16..20) != Some(expected.as_slice()) {
                return Err(ResizeError::Encode("The embedded color profile does not match the decoded pixels; convert to sRGB in an external editor before saving PNG".into()));
            }
            info.icc_profile = Some(std::borrow::Cow::Owned(icc));
        }
        if source_is_png {
            let reader = png::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(source).map_err(|e| ResizeError::Decode(e.to_string()))?,
            ))
            .read_info()
            .map_err(|e| ResizeError::Decode(e.to_string()))?;
            let source_info = reader.info();
            info.source_gamma = source_info.gamma();
            info.source_chromaticities = source_info.chromaticities();
            info.srgb = source_info.srgb;
            info.coding_independent_code_points = source_info.coding_independent_code_points;
            info.mastering_display_color_volume = source_info.mastering_display_color_volume;
            info.content_light_level = source_info.content_light_level;
        }
    }
    let mut bytes = Vec::new();
    {
        let mut writer = png::Encoder::with_info(&mut bytes, info)
            .map_err(|e| ResizeError::Encode(e.to_string()))?
            .write_header()
            .map_err(|e| ResizeError::Encode(e.to_string()))?;
        // PNG stores 16-bit samples big-endian; DynamicImage keeps native-endian.
        if sixteen {
            let data: Vec<_> = image
                .as_bytes()
                .chunks_exact(2)
                .flat_map(|v| u16::from_ne_bytes([v[0], v[1]]).to_be_bytes())
                .collect();
            writer
                .write_image_data(&data)
                .map_err(|e| ResizeError::Encode(e.to_string()))?;
        } else {
            writer
                .write_image_data(image.as_bytes())
                .map_err(|e| ResizeError::Encode(e.to_string()))?;
        }
        writer
            .finish()
            .map_err(|e| ResizeError::Encode(e.to_string()))?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn directory(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("sharpr-color-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn png_retains_linear_gamma_and_chromaticities_without_an_icc_profile() {
        let dir = directory("gamma");
        let source = dir.join("linear.png");
        let pixels = image::RgbImage::from_fn(32, 18, |x, y| image::Rgb([x as u8, y as u8, 120]));
        let mut encoder = png::Encoder::new(std::fs::File::create(&source).unwrap(), 32, 18);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_source_gamma(png::ScaledFloat::new(1.0));
        encoder.set_source_chromaticities(png::SourceChromaticities::new(
            (0.3127, 0.3290),
            (0.6400, 0.3300),
            (0.3000, 0.6000),
            (0.1500, 0.0600),
        ));
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels.as_raw()).unwrap();
        writer.finish().unwrap();
        let decoded = super::super::decode_oriented(&source).unwrap();
        for format in [OutputFormat::Png, OutputFormat::Original] {
            let bytes = format.encode(&source, &decoded, true, 100).unwrap();
            let reader = png::Decoder::new(std::io::Cursor::new(&bytes))
                .read_info()
                .unwrap();
            assert_eq!(reader.info().gamma(), Some(png::ScaledFloat::new(1.0)));
            assert!(reader.info().chromaticities().is_some());
            assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgb8(), pixels);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn png_rejects_a_cmyk_profile_attached_to_decoded_rgb_samples() {
        let dir = directory("icc");
        let source = dir.join("incompatible.jpg");
        let mut icc = include_bytes!("../../tests/assets/srgb.icc").to_vec();
        icc[16..20].copy_from_slice(b"CMYK");
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            std::fs::File::create(&source).unwrap(),
            100,
        );
        encoder.set_icc_profile(icc).unwrap();
        let pixels = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            32,
            18,
            image::Rgb([20, 30, 40]),
        ));
        encoder.encode_image(&pixels).unwrap();
        let result = OutputFormat::Png.encode(&source, &pixels, true, 100);
        assert!(
            matches!(result, Err(ResizeError::Encode(message)) if message.contains("color profile"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
