use std::path::PathBuf;

use async_channel::{Receiver, Sender};

/// Message sent from the UI to the worker pool.
pub enum WorkerRequest {
    Thumbnail(PathBuf),
    Shutdown,
}

/// Message sent back from a worker thread to the GTK main thread.
pub struct ThumbnailResult {
    pub path: PathBuf,
    /// Raw RGBA bytes at thumbnail resolution.
    pub rgba_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Manages a pool of background threads that decode and resize images.
///
/// The caller owns the `result_rx` receiver and polls it via
/// `glib::idle_add_once` / `glib::MainContext::spawn_local`.
pub struct ThumbnailWorker {
    request_tx: Sender<WorkerRequest>,
}

/// Target height for generated thumbnails (in pixels).
const THUMB_HEIGHT: u32 = 160;

impl ThumbnailWorker {
    /// Spawn `thread_count` background workers.
    /// Returns the worker handle and the receiver for results.
    pub fn spawn(thread_count: usize) -> (Self, Receiver<ThumbnailResult>) {
        let (request_tx, request_rx) = async_channel::unbounded::<WorkerRequest>();
        let (result_tx, result_rx) = async_channel::unbounded::<ThumbnailResult>();

        for _ in 0..thread_count {
            let request_rx = request_rx.clone();
            let result_tx = result_tx.clone();
            std::thread::spawn(move || {
                // Block on the async channel using a simple loop.
                loop {
                    match request_rx.recv_blocking() {
                        Ok(WorkerRequest::Thumbnail(path)) => {
                            if let Some(result) = generate_thumbnail(&path) {
                                // Ignore send errors (receiver dropped = app shutting down).
                                let _ = result_tx.send_blocking(result);
                            }
                        }
                        Ok(WorkerRequest::Shutdown) | Err(_) => break,
                    }
                }
            });
        }

        (Self { request_tx }, result_rx)
    }

    /// Queue a thumbnail request. Non-blocking; drops silently if channel is full.
    pub fn request(&self, path: PathBuf) {
        let _ = self.request_tx.send_blocking(WorkerRequest::Thumbnail(path));
    }
}

impl Drop for ThumbnailWorker {
    fn drop(&mut self) {
        // Signal all threads to stop.
        for _ in 0..8 {
            let _ = self
                .request_tx
                .send_blocking(WorkerRequest::Shutdown);
        }
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust thumbnail generation (runs on worker threads, no GTK calls)
// ---------------------------------------------------------------------------

fn generate_thumbnail(path: &PathBuf) -> Option<ThumbnailResult> {
    use image::imageops::FilterType;
    use image::ImageReader;
    use std::io::BufReader;
    use std::fs::File;

    let file = File::open(path).ok()?;
    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;

    let orig_width = img.width();
    let orig_height = img.height();
    if orig_height == 0 {
        return None;
    }

    // Scale maintaining aspect ratio so height == THUMB_HEIGHT.
    let scale = THUMB_HEIGHT as f32 / orig_height as f32;
    let thumb_width = ((orig_width as f32) * scale).round() as u32;
    let thumb_height = THUMB_HEIGHT;

    let resized = img.resize(thumb_width, thumb_height, FilterType::Lanczos3);
    let rgba = resized.into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let bytes = rgba.into_raw();

    Some(ThumbnailResult {
        path: path.clone(),
        rgba_bytes: bytes,
        width: w,
        height: h,
    })
}
