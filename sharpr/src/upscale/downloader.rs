use std::io::{Read, Write};
use std::path::Path;

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

    let mut reader = response.into_reader();
    match download_to_path(&mut reader, content_length, &dest, send) {
        Ok(()) => {}
        Err(message) => {
            let _ = tx.send_blocking(DownloadEvent::Failed(message));
        }
    }
}

fn download_to_path<R, F>(
    reader: &mut R,
    content_length: Option<u64>,
    dest: &Path,
    mut send: F,
) -> Result<(), String>
where
    R: Read,
    F: FnMut(DownloadEvent),
{
    let tmp = dest.with_extension("onnx.download");
    let mut file =
        std::fs::File::create(&tmp).map_err(|e| format!("Cannot create temp file: {e}"))?;
    let mut buf = vec![0u8; 65536];
    let mut received: u64 = 0;

    let result = (|| -> Result<(), String> {
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("Write failed: {e}"))?;
            received += n as u64;
            let progress = content_length
                .map(|total| received as f32 / total as f32)
                .unwrap_or(0.0);
            send(DownloadEvent::Progress(progress.clamp(0.0, 1.0)));
        }

        drop(file);

        std::fs::rename(&tmp, dest).map_err(|e| format!("Failed to finalise model file: {e}"))?;
        send(DownloadEvent::Done);
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("sharpr-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct InterruptingReader {
        first_chunk: Vec<u8>,
        delivered_first: bool,
    }

    impl Read for InterruptingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.delivered_first {
                return Err(io::Error::other("network cut"));
            }
            let len = self.first_chunk.len().min(buf.len());
            buf[..len].copy_from_slice(&self.first_chunk[..len]);
            self.delivered_first = true;
            Ok(len)
        }
    }

    #[test]
    fn download_to_path_writes_file_and_reports_fractional_progress() {
        let dir = temp_dir("download-success");
        let dest = dir.join("model.onnx");
        let mut reader = io::Cursor::new(b"abcdef".to_vec());
        let mut events = Vec::new();

        download_to_path(&mut reader, Some(6), &dest, |event| events.push(event)).unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"abcdef");
        assert!(
            matches!(events.as_slice(), [DownloadEvent::Progress(p), DownloadEvent::Done] if (*p - 1.0).abs() < f32::EPSILON)
        );

        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_to_path_emits_zero_progress_without_content_length() {
        let dir = temp_dir("download-no-length");
        let dest = dir.join("model.onnx");
        let mut reader = io::Cursor::new(b"abc".to_vec());
        let mut events = Vec::new();

        download_to_path(&mut reader, None, &dest, |event| events.push(event)).unwrap();

        assert!(
            matches!(events.as_slice(), [DownloadEvent::Progress(p), DownloadEvent::Done] if (*p - 0.0).abs() < f32::EPSILON)
        );

        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_to_path_cleans_up_temp_file_after_read_error() {
        let dir = temp_dir("download-read-error");
        let dest = dir.join("model.onnx");
        let tmp = dest.with_extension("onnx.download");
        let mut reader = InterruptingReader {
            first_chunk: b"abc".to_vec(),
            delivered_first: false,
        };

        let err = download_to_path(&mut reader, Some(6), &dest, |_| {}).unwrap_err();

        assert!(err.contains("Download interrupted"));
        assert!(!tmp.exists());
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_to_path_reports_finalise_failure_and_removes_temp_file() {
        let dir = temp_dir("download-finalise-error");
        let dest = dir.join("existing-dir");
        std::fs::create_dir_all(&dest).unwrap();
        let tmp = dest.with_extension("onnx.download");
        let mut reader = io::Cursor::new(b"abc".to_vec());

        let err = download_to_path(&mut reader, Some(3), &dest, |_| {}).unwrap_err();

        assert!(err.contains("Failed to finalise model file"));
        assert!(!tmp.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
