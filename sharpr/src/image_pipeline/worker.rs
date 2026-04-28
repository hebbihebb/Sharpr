//! Background workers for the shared image pipeline.
//! `PreviewWorker` runs a pool of 2 decode threads over an unbounded request
//! channel and skips stale generations both before and after decode work.
//! `MetadataWorker` runs a single EXIF metadata thread and applies the same
//! generation-aware stale-result filtering.
//! Spawn both once at window startup, store them in `SharprWindow::imp`, and
//! wire their handles into the viewer via `set_preview_worker` and
//! `set_metadata_worker`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_channel::{Receiver, Sender};

use crate::metadata::ImageMetadata;

use super::{decode_preview, PreviewDecodeError, PreviewDecodeMode, PreviewImage};

pub struct PreviewRequest {
    pub path: PathBuf,
    pub gen: u64,
}

pub struct PreviewResult {
    pub path: PathBuf,
    pub gen: u64,
    pub image: Result<PreviewImage, PreviewDecodeError>,
}

/// A cheap-to-clone handle for submitting preview decode requests to the worker pool.
#[derive(Clone)]
pub struct PreviewHandle {
    tx: Sender<PreviewRequest>,
    current_gen: Arc<AtomicU64>,
}

impl PreviewHandle {
    /// Submit a decode request, updating the shared generation so workers can
    /// skip any older pending requests before they start decode work.
    pub fn request(&self, path: PathBuf, gen: u64) {
        self.current_gen.store(gen, Ordering::Relaxed);
        let _ = self.tx.try_send(PreviewRequest { path, gen });
    }
}

/// Pool of background decode threads for full-resolution preview images.
/// Store one instance in `SharprWindow::imp` and wire the result receiver
/// to the viewer via `ViewerPane::set_preview_worker`.
pub struct PreviewWorker {
    handle: PreviewHandle,
}

const PREVIEW_WORKER_COUNT: usize = 2;

impl PreviewWorker {
    /// Spawn the worker pool. Returns the worker handle and a receiver for results.
    pub fn spawn() -> (Self, Receiver<PreviewResult>) {
        let (req_tx, req_rx) = async_channel::unbounded::<PreviewRequest>();
        let (result_tx, result_rx) = async_channel::unbounded::<PreviewResult>();
        let current_gen = Arc::new(AtomicU64::new(0));

        for _ in 0..PREVIEW_WORKER_COUNT {
            let rx = req_rx.clone();
            let tx = result_tx.clone();
            let gen_arc = current_gen.clone();
            std::thread::spawn(move || {
                while let Ok(req) = rx.recv_blocking() {
                    if req.gen != gen_arc.load(Ordering::Relaxed) {
                        crate::bench_event!(
                            "preview.skip_stale",
                            serde_json::json!({
                                "path": req.path.display().to_string(),
                                "request_gen": req.gen,
                                "current_gen": gen_arc.load(Ordering::Relaxed),
                            }),
                        );
                        continue;
                    }
                    let started = Instant::now();
                    let image = decode_preview(&req.path, PreviewDecodeMode::Viewer);
                    // Check again — user may have navigated during decode.
                    if req.gen != gen_arc.load(Ordering::Relaxed) {
                        crate::bench_event!(
                            "preview.stale_result",
                            serde_json::json!({
                                "path": req.path.display().to_string(),
                                "gen": req.gen,
                                "duration_ms": crate::bench::duration_ms(started),
                            }),
                        );
                        continue;
                    }
                    crate::bench_event!(
                        "preview.decode_finish",
                        serde_json::json!({
                            "path": req.path.display().to_string(),
                            "gen": req.gen,
                            "success": image.is_ok(),
                            "duration_ms": crate::bench::duration_ms(started),
                        }),
                    );
                    let _ = tx.send_blocking(PreviewResult {
                        path: req.path,
                        gen: req.gen,
                        image,
                    });
                }
            });
        }

        let handle = PreviewHandle {
            tx: req_tx,
            current_gen,
        };
        (Self { handle }, result_rx)
    }

    /// Return a cloneable request handle to pass to the viewer.
    pub fn handle(&self) -> PreviewHandle {
        self.handle.clone()
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        self.handle.tx.close();
    }
}

// ---------------------------------------------------------------------------
// MetadataWorker
// ---------------------------------------------------------------------------

pub struct MetadataRequest {
    pub path: PathBuf,
    pub gen: u64,
}

pub struct MetadataResult {
    pub gen: u64,
    pub metadata: ImageMetadata,
}

/// Cheap-to-clone handle for submitting metadata load requests.
#[derive(Clone)]
pub struct MetadataHandle {
    tx: Sender<MetadataRequest>,
    current_gen: Arc<AtomicU64>,
}

impl MetadataHandle {
    pub fn request(&self, path: PathBuf, gen: u64) {
        self.current_gen.store(gen, Ordering::Relaxed);
        let _ = self.tx.try_send(MetadataRequest { path, gen });
    }
}

/// Single background thread for EXIF/metadata loading.
/// Store in `SharprWindow::imp`; wire the result receiver to the viewer
/// via `ViewerPane::set_metadata_worker`.
pub struct MetadataWorker {
    handle: MetadataHandle,
}

impl MetadataWorker {
    pub fn spawn() -> (Self, Receiver<MetadataResult>) {
        let (req_tx, req_rx) = async_channel::unbounded::<MetadataRequest>();
        let (result_tx, result_rx) = async_channel::unbounded::<MetadataResult>();
        let current_gen = Arc::new(AtomicU64::new(0));
        let gen_arc = current_gen.clone();

        std::thread::spawn(move || {
            while let Ok(req) = req_rx.recv_blocking() {
                if req.gen != gen_arc.load(Ordering::Relaxed) {
                    continue;
                }
                let metadata = ImageMetadata::load(&req.path);
                if req.gen != gen_arc.load(Ordering::Relaxed) {
                    continue;
                }
                let _ = result_tx.send_blocking(MetadataResult {
                    gen: req.gen,
                    metadata,
                });
            }
        });

        let handle = MetadataHandle {
            tx: req_tx,
            current_gen,
        };
        (Self { handle }, result_rx)
    }

    pub fn handle(&self) -> MetadataHandle {
        self.handle.clone()
    }
}

impl Drop for MetadataWorker {
    fn drop(&mut self) {
        self.handle.tx.close();
    }
}
