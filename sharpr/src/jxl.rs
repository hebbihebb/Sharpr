use std::path::Path;

use image::DynamicImage;
use jpegxl_rs::encode::{EncoderFrame, EncoderResult, EncoderSpeed};
use jpegxl_rs::image::ToDynamic;
use jpegxl_rs::{decoder_builder, encoder_builder};

pub fn is_jxl_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("jxl"))
        .unwrap_or(false)
}

pub fn decode_path(path: &Path) -> Result<DynamicImage, String> {
    let data = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    decoder_builder()
        .build()
        .map_err(|err| format!("create JPEG XL decoder: {err}"))?
        .decode_to_image(&data)
        .map_err(|err| format!("decode JPEG XL {}: {err}", path.display()))?
        .ok_or_else(|| format!("decode JPEG XL {}: no image data returned", path.display()))
}

pub fn image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let data = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let (metadata, _) = decoder_builder()
        .build()
        .map_err(|err| format!("create JPEG XL decoder: {err}"))?
        .decode(&data)
        .map_err(|err| format!("decode JPEG XL metadata {}: {err}", path.display()))?;
    Ok((metadata.width, metadata.height))
}

pub fn encode_path(
    image: &DynamicImage,
    output: &Path,
    quality: u8,
    lossless: bool,
    effort: u8,
) -> Result<(), String> {
    let quality = quality.clamp(0, 100) as f32;
    let mut encoder = encoder_builder()
        .has_alpha(image.color().has_alpha())
        .lossless(lossless)
        .speed(speed_for_effort(effort))
        .jpeg_quality(if lossless { 100.0 } else { quality })
        .build()
        .map_err(|err| format!("create JPEG XL encoder: {err}"))?;

    let encoded: EncoderResult<u8> = if image.color().has_alpha() {
        let rgba = image.to_rgba8();
        encoder
            .encode_frame(
                &EncoderFrame::new(rgba.as_raw()).num_channels(4),
                rgba.width(),
                rgba.height(),
            )
            .map_err(|err| format!("encode JPEG XL {}: {err}", output.display()))?
    } else {
        let rgb = image.to_rgb8();
        encoder
            .encode(rgb.as_raw(), rgb.width(), rgb.height())
            .map_err(|err| format!("encode JPEG XL {}: {err}", output.display()))?
    };

    std::fs::write(output, encoded.data)
        .map_err(|err| format!("write JPEG XL {}: {err}", output.display()))
}

fn speed_for_effort(effort: u8) -> EncoderSpeed {
    match effort.clamp(1, 10) {
        1 => EncoderSpeed::Lightning,
        2 => EncoderSpeed::Thunder,
        3 => EncoderSpeed::Falcon,
        4 => EncoderSpeed::Cheetah,
        5 => EncoderSpeed::Hare,
        6 => EncoderSpeed::Wombat,
        7 => EncoderSpeed::Squirrel,
        8 => EncoderSpeed::Kitten,
        9 => EncoderSpeed::Tortoise,
        _ => EncoderSpeed::Glacier,
    }
}
