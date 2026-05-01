use std::path::Path;
use std::time::Instant;

use image::DynamicImage;
use jpegxl_rs::encode::{EncoderFrame, EncoderResult, EncoderSpeed};
use jpegxl_rs::image::ToDynamic;
use jpegxl_rs::parallel::threads_runner::ThreadsRunner;
use jpegxl_rs::{decoder_builder, encoder_builder};

const DEFAULT_DECODE_WORKERS: usize = 2;

pub fn is_jxl_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("jxl"))
        .unwrap_or(false)
}

pub fn decode_path(path: &Path) -> Result<DynamicImage, String> {
    decode_path_with_num_workers(path, DEFAULT_DECODE_WORKERS)
}

fn decode_path_with_num_workers(path: &Path, num_workers: usize) -> Result<DynamicImage, String> {
    let data = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let parallel_runner = ThreadsRunner::new(None, Some(num_workers.max(1)))
        .ok_or_else(|| "create JPEG XL thread pool".to_string())?;
    let decoder = decoder_builder()
        .parallel_runner(&parallel_runner)
        .build()
        .map_err(|err| format!("create JPEG XL decoder: {err}"))?;
    decoder
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
    let started = Instant::now();
    let quality = quality.clamp(0, 100) as f32;
    let parallel_runner = ThreadsRunner::new(None, None)
        .ok_or_else(|| "create JPEG XL thread pool".to_string())?;
    let mut encoder = encoder_builder()
        .has_alpha(image.color().has_alpha())
        .lossless(lossless)
        .speed(speed_for_effort(effort))
        .jpeg_quality(if lossless { 100.0 } else { quality })
        .parallel_runner(&parallel_runner)
        .build()
        .map_err(|err| format!("create JPEG XL encoder: {err}"))?;
    crate::bench_event!(
        "jxl.encode.start",
        serde_json::json!({
            "output": output.display().to_string(),
            "width": image.width(),
            "height": image.height(),
            "has_alpha": image.color().has_alpha(),
            "quality": quality,
            "lossless": lossless,
            "effort": effort,
        }),
    );

    let encoded: EncoderResult<u8> = if image.color().has_alpha() {
        let rgba_started = Instant::now();
        let rgba = image.to_rgba8();
        let rgba_ms = crate::bench::duration_ms(rgba_started);
        let encode_started = Instant::now();
        let result = encoder
            .encode_frame(
                &EncoderFrame::new(rgba.as_raw()).num_channels(4),
                rgba.width(),
                rgba.height(),
            )
            .map_err(|err| format!("encode JPEG XL {}: {err}", output.display()))?;
        crate::bench_event!(
            "jxl.encode.stage",
            serde_json::json!({
                "output": output.display().to_string(),
                "pixel_prep_ms": rgba_ms,
                "encode_ms": crate::bench::duration_ms(encode_started),
                "path": "rgba",
            }),
        );
        result
    } else {
        let rgb_started = Instant::now();
        let rgb = image.to_rgb8();
        let rgb_ms = crate::bench::duration_ms(rgb_started);
        let encode_started = Instant::now();
        let result = encoder
            .encode(rgb.as_raw(), rgb.width(), rgb.height())
            .map_err(|err| format!("encode JPEG XL {}: {err}", output.display()))?;
        crate::bench_event!(
            "jxl.encode.stage",
            serde_json::json!({
                "output": output.display().to_string(),
                "pixel_prep_ms": rgb_ms,
                "encode_ms": crate::bench::duration_ms(encode_started),
                "path": "rgb",
            }),
        );
        result
    };

    let write_started = Instant::now();
    std::fs::write(output, encoded.data)
        .map_err(|err| format!("write JPEG XL {}: {err}", output.display()))?;
    crate::bench_event!(
        "jxl.encode.done",
        serde_json::json!({
            "output": output.display().to_string(),
            "write_ms": crate::bench::duration_ms(write_started),
            "duration_ms": crate::bench::duration_ms(started),
        }),
    );
    Ok(())
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
