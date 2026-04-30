use std::path::{Path, PathBuf};

use crate::upscale::{UpscaleCompressionMode, UpscaleJobConfig, UpscaleOutputFormat};

/// Result sent back to the main thread after an upscale job completes.
pub enum UpscaleEvent {
    /// Fraction complete in [0.0, 1.0]; `None` means pulse (indeterminate).
    Progress(Option<f32>),
    /// Job finished successfully. Contains path to the output file.
    Done(PathBuf),
    /// Job failed with an error message.
    Failed(String),
}

/// Phase B — AI upscaling subprocess runner.
///
/// Wraps `gio::Subprocess` invocation of the Vulkan upscaler and streams
/// `UpscaleEvent` values back to the GTK main thread via an async-channel.
pub struct UpscaleRunner;

impl UpscaleRunner {
    /// Spawn an upscale job.
    pub fn run(
        binary: &Path,
        input: &Path,
        output: &Path,
        config: UpscaleJobConfig,
    ) -> async_channel::Receiver<UpscaleEvent> {
        let (tx, rx) = async_channel::bounded::<UpscaleEvent>(64);

        let binary = binary.to_path_buf();
        let input = input.to_path_buf();
        let output = output.to_path_buf();

        std::thread::spawn(move || {
            run_subprocess(binary, input, output, config, tx);
        });

        rx
    }

    /// Compute a conservative requested scale factor from the source image dimensions.
    pub fn smart_scale(width: u32, height: u32) -> u32 {
        let long_edge = width.max(height);
        if long_edge == 0 {
            return 2;
        }
        match long_edge {
            0..=1199 => 4,
            1200..=2199 => 3,
            _ => 2,
        }
    }

    pub fn select_output_format(config: &UpscaleJobConfig) -> UpscaleOutputFormat {
        if config.compress_output {
            config.compressed_format
        } else {
            UpscaleOutputFormat::Png
        }
    }
}

fn run_subprocess(
    binary: PathBuf,
    input: PathBuf,
    output: PathBuf,
    config: UpscaleJobConfig,
    tx: async_channel::Sender<UpscaleEvent>,
) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let send = |ev: UpscaleEvent| {
        let _ = tx.send_blocking(ev);
    };

    let models_dir = binary
        .parent()
        .map(|p| p.join("models"))
        .unwrap_or_else(|| PathBuf::from("models"));
    let intermediate = intermediate_output_path(&output);

    let mut command = Command::new(&binary);
    command.args([
        "-i",
        &input.to_string_lossy(),
        "-o",
        &intermediate.to_string_lossy(),
        "-s",
        &config.execution_scale.to_string(),
        "-n",
        config.model.model_name(),
        "-m",
        &models_dir.to_string_lossy(),
        "-f",
        "png",
    ]);
    if let Some(tile_size) = config.tile_size {
        command.args(["-t", &tile_size.to_string()]);
    }
    if let Some(gpu_id) = config.gpu_id {
        command.args(["-g", &gpu_id.to_string()]);
    }

    let mut child = match command.stderr(Stdio::piped()).stdout(Stdio::null()).spawn() {
        Ok(c) => c,
        Err(e) => {
            send(UpscaleEvent::Failed(format!(
                "Failed to start upscaler: {e}"
            )));
            return;
        }
    };

    let stderr = child.stderr.take().expect("stderr was piped");
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        send(UpscaleEvent::Progress(parse_progress(&line)));
    }

    match child.wait() {
        Ok(status) if status.success() => {
            send(UpscaleEvent::Progress(None));
            match finalize_output(&intermediate, &output, config) {
                Ok(()) => send(UpscaleEvent::Done(output)),
                Err(err) => send(UpscaleEvent::Failed(err)),
            }
        }
        Ok(status) => send(UpscaleEvent::Failed(format!(
            "Upscaler exited with status {}",
            status
        ))),
        Err(e) => send(UpscaleEvent::Failed(format!("Upscaler I/O error: {e}"))),
    }
}

pub(crate) fn finalize_output(
    intermediate: &Path,
    output: &Path,
    config: UpscaleJobConfig,
) -> Result<(), String> {
    let result = finalize_output_without_cleanup(intermediate, output, config);
    let _ = std::fs::remove_file(intermediate); // always clean up temp file
    result
}

pub(crate) fn finalize_output_without_cleanup(
    intermediate: &Path,
    output: &Path,
    config: UpscaleJobConfig,
) -> Result<(), String> {
    use image::imageops::FilterType;
    use image::ImageReader;
    use std::fs::File;
    use std::io::BufReader;

    let target_format = UpscaleRunner::select_output_format(&config);
    let (input_w, input_h) = config.source_dimensions;
    if input_w == 0 || input_h == 0 {
        return Err("source dimensions are unavailable".to_string());
    }
    let target_w = input_w
        .checked_mul(config.requested_scale)
        .ok_or_else(|| "target width overflowed".to_string())?;
    let target_h = input_h
        .checked_mul(config.requested_scale)
        .ok_or_else(|| "target height overflowed".to_string())?;

    let reader = ImageReader::new(BufReader::new(
        File::open(intermediate)
            .map_err(|err| format!("failed to open intermediate output: {err}"))?,
    ))
    .with_guessed_format()
    .map_err(|err| format!("failed to detect intermediate format: {err}"))?;
    let intermediate_image = reader
        .decode()
        .map_err(|err| format!("failed to decode intermediate output: {err}"))?;

    let final_image =
        if intermediate_image.width() == target_w && intermediate_image.height() == target_h {
            intermediate_image
        } else {
            intermediate_image.resize_exact(target_w, target_h, FilterType::Lanczos3)
        };

    if config.compress_output && config.keep_raw_png_sidecar {
        save_image(
            final_image.clone(),
            &preserved_png_temp_path(output),
            UpscaleOutputFormat::Png,
            UpscaleCompressionMode::Lossless,
            100,
        )?;
    }

    save_image(
        final_image,
        output,
        target_format,
        config.compression_mode,
        config.quality,
    )
}

pub(crate) fn preserved_png_temp_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("upscaled");
    output.with_file_name(format!("{stem}.preserved.png"))
}

pub(crate) fn save_image(
    image: image::DynamicImage,
    output: &Path,
    format: UpscaleOutputFormat,
    compression_mode: UpscaleCompressionMode,
    quality: u8,
) -> Result<(), String> {
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    use std::io::{BufWriter, Write};

    match format {
        UpscaleOutputFormat::Jpeg => {
            let file = std::fs::File::create(output)
                .map_err(|err| format!("failed to create output {}: {err}", output.display()))?;
            let writer = BufWriter::new(file);
            let rgb = image.to_rgb8();
            let encoder = JpegEncoder::new_with_quality(writer, quality.clamp(50, 100));
            encoder
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|err| format!("failed to encode JPEG output: {err}"))
        }
        UpscaleOutputFormat::Jxl => crate::jxl::encode_path(
            &image,
            output,
            quality.clamp(50, 100),
            matches!(compression_mode, UpscaleCompressionMode::Lossless),
            7,
        ),
        UpscaleOutputFormat::Png => {
            let file = std::fs::File::create(output)
                .map_err(|err| format!("failed to create output {}: {err}", output.display()))?;
            let writer = BufWriter::new(file);
            let rgba = image.to_rgba8();
            let encoder = PngEncoder::new_with_quality(
                writer,
                match compression_mode {
                    UpscaleCompressionMode::Lossless => CompressionType::Best,
                    UpscaleCompressionMode::Auto | UpscaleCompressionMode::Lossy => {
                        CompressionType::Default
                    }
                },
                FilterType::Adaptive,
            );
            encoder
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|err| format!("failed to encode PNG output: {err}"))
        }
        UpscaleOutputFormat::Webp => {
            let file = std::fs::File::create(output)
                .map_err(|err| format!("failed to create output {}: {err}", output.display()))?;
            let mut writer = BufWriter::new(file);
            if image.color().has_alpha() {
                let rgba = image.to_rgba8();
                let encoded = if matches!(compression_mode, UpscaleCompressionMode::Lossless) {
                    webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                        .encode_lossless()
                } else {
                    webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                        .encode(quality.clamp(1, 100) as f32)
                };
                writer
                    .write_all(encoded.as_ref())
                    .map_err(|err| format!("failed to write WebP output: {err}"))
            } else {
                let rgb = image.to_rgb8();
                let encoded = if matches!(compression_mode, UpscaleCompressionMode::Lossless) {
                    webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height())
                        .encode_lossless()
                } else {
                    webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height())
                        .encode(quality.clamp(1, 100) as f32)
                };
                writer
                    .write_all(encoded.as_ref())
                    .map_err(|err| format!("failed to write WebP output: {err}"))
            }
        }
    }
}

fn intermediate_output_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("upscaled");
    output.with_file_name(format!("{stem}.ncnn-intermediate.png"))
}

/// Parse a fraction [0, 1] from an NCNN progress line.
/// Recognises "N/M" and "N%" patterns; returns `None` for pulse.
fn parse_progress(line: &str) -> Option<f32> {
    if let Some(slash) = line.find('/') {
        let numer: f32 = line[..slash].trim().parse().ok()?;
        let denom: f32 = line[slash + 1..].trim().parse().ok()?;
        if denom > 0.0 {
            return Some((numer / denom).clamp(0.0, 1.0));
        }
    }
    if let Some(pct_pos) = line.find('%') {
        let val: f32 = line[..pct_pos].trim().parse().ok()?;
        return Some((val / 100.0).clamp(0.0, 1.0));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, ImageFormat, Rgba};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sharpr-upscale-runner-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_config(format: UpscaleOutputFormat) -> UpscaleJobConfig {
        UpscaleJobConfig {
            source_dimensions: (4, 3),
            requested_scale: 2,
            execution_scale: 2,
            model: crate::upscale::UpscaleModel::Standard,
            compress_output: format != UpscaleOutputFormat::Png,
            compressed_format: format,
            keep_raw_png_sidecar: false,
            compression_mode: UpscaleCompressionMode::Lossy,
            quality: 80,
            tile_size: None,
            gpu_id: None,
        }
    }

    #[test]
    fn finalize_output_writes_valid_webp() {
        let dir = temp_dir("finalize-webp");
        let input = dir.join("input.png");
        let intermediate = dir.join("intermediate.png");
        let output = dir.join("output.webp");

        let src = ImageBuffer::from_fn(4, 3, |x, y| {
            Rgba([(x * 10) as u8, (y * 20) as u8, 200, 255])
        });
        src.save_with_format(&input, ImageFormat::Png).unwrap();

        let upscaled =
            ImageBuffer::from_fn(8, 6, |x, y| Rgba([255, (x * 5) as u8, (y * 7) as u8, 255]));
        upscaled
            .save_with_format(&intermediate, ImageFormat::Png)
            .unwrap();

        finalize_output(
            &intermediate,
            &output,
            sample_config(UpscaleOutputFormat::Webp),
        )
        .unwrap();

        assert!(!intermediate.exists());
        let decoded = image::open(&output).unwrap();
        assert_eq!(decoded.width(), 8);
        assert_eq!(decoded.height(), 6);
        assert_eq!(
            image::ImageReader::open(&output)
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .format(),
            Some(ImageFormat::WebP)
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finalize_output_without_cleanup_keeps_intermediate() {
        let dir = temp_dir("keep-intermediate");
        let input = dir.join("input.png");
        let intermediate = dir.join("intermediate.png");
        let output = dir.join("output.webp");

        let src = ImageBuffer::from_fn(4, 3, |x, y| {
            Rgba([(x * 10) as u8, (y * 20) as u8, 200, 255])
        });
        src.save_with_format(&input, ImageFormat::Png).unwrap();

        let upscaled =
            ImageBuffer::from_fn(8, 6, |x, y| Rgba([255, (x * 5) as u8, (y * 7) as u8, 255]));
        upscaled
            .save_with_format(&intermediate, ImageFormat::Png)
            .unwrap();

        finalize_output_without_cleanup(
            &intermediate,
            &output,
            sample_config(UpscaleOutputFormat::Webp),
        )
        .unwrap();

        assert!(intermediate.exists());
        assert_eq!(
            image::ImageReader::open(&output)
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .format(),
            Some(ImageFormat::WebP)
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finalize_output_writes_preserved_png_sidecar_for_lossy_output() {
        let dir = temp_dir("preserved-sidecar");
        let input = dir.join("input.png");
        let intermediate = dir.join("intermediate.png");
        let output = dir.join("output.webp");
        let preserved = preserved_png_temp_path(&output);

        let src = ImageBuffer::from_fn(4, 3, |x, y| {
            Rgba([(x * 10) as u8, (y * 20) as u8, 200, 255])
        });
        src.save_with_format(&input, ImageFormat::Png).unwrap();

        let upscaled =
            ImageBuffer::from_fn(8, 6, |x, y| Rgba([255, (x * 5) as u8, (y * 7) as u8, 255]));
        upscaled
            .save_with_format(&intermediate, ImageFormat::Png)
            .unwrap();

        let mut config = sample_config(UpscaleOutputFormat::Webp);
        config.keep_raw_png_sidecar = true;

        finalize_output_without_cleanup(&intermediate, &output, config).unwrap();

        assert!(preserved.exists());
        assert_eq!(
            image::ImageReader::open(&preserved)
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .format(),
            Some(ImageFormat::Png)
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finalize_output_skips_preserved_png_sidecar_by_default() {
        let dir = temp_dir("no-sidecar");
        let intermediate = dir.join("intermediate.png");
        let output = dir.join("output.jxl");
        let preserved = preserved_png_temp_path(&output);

        let upscaled =
            ImageBuffer::from_fn(8, 6, |x, y| Rgba([255, (x * 5) as u8, (y * 7) as u8, 255]));
        upscaled
            .save_with_format(&intermediate, ImageFormat::Png)
            .unwrap();

        finalize_output_without_cleanup(
            &intermediate,
            &output,
            sample_config(UpscaleOutputFormat::Jxl),
        )
        .unwrap();

        assert!(!preserved.exists());

        let _ = std::fs::remove_dir_all(dir);
    }
}
