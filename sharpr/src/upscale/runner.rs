use std::path::{Path, PathBuf};

use crate::upscale::UpscaleModel;

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
/// Wraps `gio::Subprocess` invocation of realesrgan-ncnn-vulkan and streams
/// `UpscaleEvent` values back to the GTK main thread via an async-channel.
pub struct UpscaleRunner;

impl UpscaleRunner {
    /// Spawn an upscale job.
    ///
    /// - `binary`: path to `realesrgan-ncnn-vulkan`
    /// - `input`: source image
    /// - `output`: destination path for the upscaled result
    /// - `scale`: integer scale factor (2–4)
    ///
    /// Returns a receiver that yields `UpscaleEvent` messages on the GLib main
    /// context. The caller should `spawn_local` a loop consuming it.
    pub fn run(
        binary: &Path,
        input: &Path,
        output: &Path,
        scale: u32,
        model: UpscaleModel,
    ) -> async_channel::Receiver<UpscaleEvent> {
        let (tx, rx) = async_channel::bounded::<UpscaleEvent>(64);

        let binary = binary.to_path_buf();
        let input = input.to_path_buf();
        let output = output.to_path_buf();

        // The subprocess is launched on a background thread so we don't need
        // an async GIO runtime. Progress lines are sent through the channel.
        std::thread::spawn(move || {
            run_subprocess(binary, input, output, scale, model, tx);
        });

        rx
    }

    /// Compute a sensible scale factor from the source image dimensions.
    /// Targets 3840 px on the long edge; clamps to [2, 4].
    pub fn smart_scale(width: u32, height: u32) -> u32 {
        let long_edge = width.max(height) as f32;
        if long_edge == 0.0 {
            return 2;
        }
        ((3840.0 / long_edge).ceil() as u32).clamp(2, 4)
    }
}

// ---------------------------------------------------------------------------
// Blocking subprocess helper (runs on a worker thread)
// ---------------------------------------------------------------------------

fn run_subprocess(
    binary: PathBuf,
    input: PathBuf,
    output: PathBuf,
    scale: u32,
    model: UpscaleModel,
    tx: async_channel::Sender<UpscaleEvent>,
) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    // Send a helper that silently drops on channel-closed errors.
    let send = |ev: UpscaleEvent| {
        let _ = tx.send_blocking(ev);
    };

    // The binary looks for model files in a `models/` directory relative to
    // its own location (not the working directory). Pass `-m` explicitly so
    // it works whether installed to ~/.local/bin or /app/bin inside Flatpak.
    let models_dir = binary
        .parent()
        .map(|p| p.join("models"))
        .unwrap_or_else(|| std::path::PathBuf::from("models"));

    let mut child = match Command::new(&binary)
        .args([
            "-i",
            &input.to_string_lossy(),
            "-o",
            &output.to_string_lossy(),
            "-s",
            &scale.to_string(),
            "-n",
            model.model_name(),
            "-m",
            &models_dir.to_string_lossy(),
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            send(UpscaleEvent::Failed(format!("Failed to start upscaler: {e}")));
            return;
        }
    };

    // Parse stderr for progress. NCNN tools typically emit lines like:
    //   "0/100" or "50%" or "50.00%"
    // Emit Progress(None) (pulse) for unrecognised lines.
    let stderr = child.stderr.take().expect("stderr was piped");
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        send(UpscaleEvent::Progress(parse_progress(&line)));
    }

    match child.wait() {
        Ok(status) if status.success() => send(UpscaleEvent::Done(output)),
        Ok(status) => send(UpscaleEvent::Failed(format!(
            "Upscaler exited with status {}",
            status
        ))),
        Err(e) => send(UpscaleEvent::Failed(format!("Upscaler I/O error: {e}"))),
    }
}

/// Parse a fraction [0, 1] from an NCNN progress line.
/// Recognises "N/M" and "N%" patterns; returns `None` for pulse.
fn parse_progress(line: &str) -> Option<f32> {
    // "N/M" pattern
    if let Some(slash) = line.find('/') {
        let numer: f32 = line[..slash].trim().parse().ok()?;
        let denom: f32 = line[slash + 1..].trim().parse().ok()?;
        if denom > 0.0 {
            return Some((numer / denom).clamp(0.0, 1.0));
        }
    }
    // "N%" or "N.N%" pattern
    if let Some(pct_pos) = line.find('%') {
        let val: f32 = line[..pct_pos].trim().parse().ok()?;
        return Some((val / 100.0).clamp(0.0, 1.0));
    }
    None
}
