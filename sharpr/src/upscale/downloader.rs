use std::io::{Read, Write};

use crate::upscale::{backends::onnx::OnnxBackend, OnnxUpscaleModel};

pub enum DownloadEvent {
    Progress(f32),
    Done,
    Failed(String),
}

/// Download `model`'s ONNX file to the standard models directory.
///
/// Streams in 64 KB chunks; emits `Progress` events when Content-Length is
/// known, otherwise emits `Progress(0.0)` pulses.
pub fn download_model(model: OnnxUpscaleModel) -> async_channel::Receiver<DownloadEvent> {
    let (tx, rx) = async_channel::bounded(64);
    std::thread::spawn(move || run_download(model, tx));
    rx
}

fn run_download(model: OnnxUpscaleModel, tx: async_channel::Sender<DownloadEvent>) {
    let send = |ev: DownloadEvent| {
        let _ = tx.send_blocking(ev);
    };

    let info = model.info();
    let dest = OnnxBackend::model_path(model);

    if let Some(dir) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            send(DownloadEvent::Failed(format!(
                "Cannot create models dir: {e}"
            )));
            return;
        }
    }

    let response = match ureq::get(info.download_url).call() {
        Ok(r) => r,
        Err(e) => {
            send(DownloadEvent::Failed(format!(
                "Download request failed: {e}"
            )));
            return;
        }
    };

    let content_length: Option<u64> = response
        .header("Content-Length")
        .and_then(|v| v.parse().ok());

    let tmp = dest.with_extension("onnx.download");
    let mut file = match std::fs::File::create(&tmp) {
        Ok(f) => f,
        Err(e) => {
            send(DownloadEvent::Failed(format!(
                "Cannot create temp file: {e}"
            )));
            return;
        }
    };

    let mut reader = response.into_reader();
    let mut buf = vec![0u8; 65536];
    let mut received: u64 = 0;

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                send(DownloadEvent::Failed(format!("Download interrupted: {e}")));
                return;
            }
        };
        if let Err(e) = file.write_all(&buf[..n]) {
            let _ = std::fs::remove_file(&tmp);
            send(DownloadEvent::Failed(format!("Write failed: {e}")));
            return;
        }
        received += n as u64;
        let progress = content_length
            .map(|total| received as f32 / total as f32)
            .unwrap_or(0.0);
        send(DownloadEvent::Progress(progress.clamp(0.0, 1.0)));
    }

    drop(file);

    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        send(DownloadEvent::Failed(format!(
            "Failed to finalise model file: {e}"
        )));
        return;
    }

    send(DownloadEvent::Done);
}
