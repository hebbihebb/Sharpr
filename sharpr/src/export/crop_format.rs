//! Crop copies retain their source container; batch export stays WebP.
use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageFormat};

use super::ResizeError;

#[derive(Clone, Copy)]
pub(super) enum CropFormat {
    Raster(ImageFormat),
    Jxl,
}

impl CropFormat {
    pub fn detect(source: &Path) -> Result<Self, ResizeError> {
        if crate::jxl::is_jxl_path(source) {
            return Ok(Self::Jxl);
        }
        let format = image::ImageReader::open(source)
            .and_then(|reader| reader.with_guessed_format())
            .map_err(|e| ResizeError::Decode(e.to_string()))?
            .format()
            .ok_or_else(|| ResizeError::Decode("unrecognized source format".into()))?;
        match format {
            ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::WebP
            | ImageFormat::Tiff
            | ImageFormat::Gif
            | ImageFormat::Bmp
            | ImageFormat::Ico => Ok(Self::Raster(format)),
            _ => Err(ResizeError::Encode(format!(
                "Crop Copy cannot save {format:?} in its original format"
            ))),
        }
    }

    pub fn extension(self, source: &Path) -> String {
        match self {
            Self::Jxl => "jxl".into(),
            Self::Raster(format) => source
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| ImageFormat::from_extension(s) == Some(format))
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| format.extensions_str()[0].into()),
        }
    }

    pub fn encode(self, image: &DynamicImage) -> Result<Vec<u8>, ResizeError> {
        let mut output = Cursor::new(Vec::new());
        match self {
            Self::Raster(ImageFormat::WebP) => return super::webp_bytes(image, true, 100),
            Self::Raster(ImageFormat::Jpeg) => {
                // JPEG cropping requires re-encoding. Use maximum quality rather
                // than the image crate's lower default; user accepted this tradeoff.
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 100)
                    .encode_image(&image.to_rgb8())
                    .map_err(|e| ResizeError::Encode(e.to_string()))?;
            }
            Self::Raster(ImageFormat::Ico) => DynamicImage::ImageRgba8(image.to_rgba8())
                .write_to(&mut output, ImageFormat::Ico)
                .map_err(|e| ResizeError::Encode(e.to_string()))?,
            Self::Raster(format) => image
                .write_to(&mut output, format)
                .map_err(|e| ResizeError::Encode(e.to_string()))?,
            Self::Jxl => return jxl_bytes(image),
        }
        Ok(output.into_inner())
    }
}

fn jxl_bytes(image: &DynamicImage) -> Result<Vec<u8>, ResizeError> {
    use jpegxl_rs::{
        encode::{EncoderFrame, EncoderResult},
        encoder_builder,
    };
    let alpha = image.color().has_alpha();
    let mut encoder = encoder_builder()
        .has_alpha(alpha)
        .lossless(true)
        .uses_original_profile(true)
        .build()
        .map_err(|e| ResizeError::Encode(e.to_string()))?;
    let channels = if alpha { 4 } else { 3 };
    if matches!(
        image.color(),
        image::ColorType::Rgb32F | image::ColorType::Rgba32F
    ) {
        let data = if alpha {
            image.to_rgba32f().into_raw()
        } else {
            image.to_rgb32f().into_raw()
        };
        let encoded: EncoderResult<f32> = encoder
            .encode_frame(
                &EncoderFrame::new(&data).num_channels(channels),
                image.width(),
                image.height(),
            )
            .map_err(|e| ResizeError::Encode(e.to_string()))?;
        return Ok(encoded.data);
    }
    let encoded: EncoderResult<u8> =
        if image.color().bits_per_pixel() / image.color().channel_count() as u16 > 8 {
            let data = if alpha {
                image.to_rgba16().into_raw()
            } else {
                image.to_rgb16().into_raw()
            };
            // The output generic determines the sample depth in libjxl.
            let encoded: EncoderResult<u16> = encoder
                .encode_frame(
                    &EncoderFrame::new(&data).num_channels(channels),
                    image.width(),
                    image.height(),
                )
                .map_err(|e| ResizeError::Encode(e.to_string()))?;
            return Ok(encoded.data);
        } else {
            let data = if alpha {
                image.to_rgba8().into_raw()
            } else {
                image.to_rgb8().into_raw()
            };
            encoder
                .encode_frame(
                    &EncoderFrame::new(&data).num_channels(channels),
                    image.width(),
                    image.height(),
                )
                .map_err(|e| ResizeError::Encode(e.to_string()))?
        };
    Ok(encoded.data)
}
