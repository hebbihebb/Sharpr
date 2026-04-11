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

    pub fn select_output_format(
        input: &Path,
        preferred: UpscaleOutputFormat,
        compression_mode: UpscaleCompressionMode,
    ) -> UpscaleOutputFormat {
        if preferred != UpscaleOutputFormat::Auto {
            return preferred;
        }

        if source_has_alpha(input).unwrap_or(false) {
            return UpscaleOutputFormat::Webp;
        }

        match compression_mode {
            UpscaleCompressionMode::Lossless => UpscaleOutputFormat::Webp,
            UpscaleCompressionMode::Auto | UpscaleCompressionMode::Lossy => {
                UpscaleOutputFormat::Jpeg
            }
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
            send(UpscaleEvent::Failed(format!("Failed to start upscaler: {e}")));
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
            match finalize_output(&input, &intermediate, &output, config) {
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

fn finalize_output(
    input: &Path,
    intermediate: &Path,
    output: &Path,
    config: UpscaleJobConfig,
) -> Result<(), String> {
    use image::imageops::FilterType;
    use image::ImageReader;
    use std::fs::File;
    use std::io::BufReader;

    let target_format =
        UpscaleRunner::select_output_format(input, config.output_format, config.compression_mode);
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

    let final_image = if intermediate_image.width() == target_w
        && intermediate_image.height() == target_h
    {
        intermediate_image
    } else {
        intermediate_image.resize_exact(target_w, target_h, FilterType::Lanczos3)
    };

    save_image(
        final_image,
        output,
        target_format,
        config.compression_mode,
        config.quality,
    )?;

    let _ = std::fs::remove_file(intermediate);
    Ok(())
}

fn save_image(
    image: image::DynamicImage,
    output: &Path,
    format: UpscaleOutputFormat,
    compression_mode: UpscaleCompressionMode,
    quality: u8,
) -> Result<(), String> {
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::codecs::webp::WebPEncoder;
    use image::{ExtendedColorType, ImageEncoder};
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(output)
        .map_err(|err| format!("failed to create output {}: {err}", output.display()))?;
    let writer = BufWriter::new(file);

    match format {
        UpscaleOutputFormat::Jpeg => {
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
        UpscaleOutputFormat::Png | UpscaleOutputFormat::Auto => {
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
            if image.color().has_alpha() {
                let rgba = image.to_rgba8();
                WebPEncoder::new_lossless(writer)
                    .write_image(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(|err| format!("failed to encode WebP output: {err}"))
            } else {
                let rgb = image.to_rgb8();
                WebPEncoder::new_lossless(writer)
                    .write_image(
                        rgb.as_raw(),
                        rgb.width(),
                        rgb.height(),
                        ExtendedColorType::Rgb8,
                    )
                    .map_err(|err| format!("failed to encode WebP output: {err}"))
            }
        }
    }
}

fn source_has_alpha(path: &Path) -> Result<bool, String> {
    use image::ImageDecoder;
    use image::ImageReader;
    use std::fs::File;
    use std::io::BufReader;

    let reader = ImageReader::new(BufReader::new(
        File::open(path).map_err(|err| format!("failed to open source image: {err}"))?,
    ))
    .with_guessed_format()
    .map_err(|err| format!("failed to detect source format: {err}"))?;
    let decoder = reader
        .into_decoder()
        .map_err(|err| format!("failed to inspect source decoder: {err}"))?;
    Ok(decoder.color_type().has_alpha())
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
