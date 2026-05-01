//! Background workers for the shared image pipeline.
//! `PreviewWorker` runs a pool of 2 decode threads over a role-aware priority
//! queue and skips stale generations independently for viewer and prefetch work.
//! `MetadataWorker` runs a single EXIF metadata thread and applies the same
//! generation-aware stale-result filtering.
//! Spawn both once at window startup, store them in `SharprWindow::imp`, and
//! wire their handles into the viewer via `set_preview_worker` and
//! `set_metadata_worker`.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use async_channel::{Receiver, Sender};

use crate::metadata::ImageMetadata;

use super::{decode_preview, PreviewDecodeError, PreviewDecodeMode, PreviewImage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewRequestRole {
    Viewer,
    Prefetch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefetchRequestContext {
    pub index: u32,
    pub direction: i32,
    pub distance: u32,
}

pub struct PreviewRequest {
    pub path: PathBuf,
    pub gen: u64,
    pub role: PreviewRequestRole,
    pub prefetch: Option<PrefetchRequestContext>,
}

pub struct PreviewResult {
    pub path: PathBuf,
    pub gen: u64,
    pub role: PreviewRequestRole,
    pub prefetch: Option<PrefetchRequestContext>,
    pub image: Result<PreviewImage, PreviewDecodeError>,
}

#[derive(Default)]
struct PreviewRequestQueue {
    viewer: VecDeque<PreviewRequest>,
    prefetch: VecDeque<PreviewRequest>,
}

impl PreviewRequestQueue {
    fn push(&mut self, request: PreviewRequest) {
        match request.role {
            PreviewRequestRole::Viewer => self.viewer.push_back(request),
            PreviewRequestRole::Prefetch => self.prefetch.push_back(request),
        }
    }

    fn pop(&mut self) -> Option<PreviewRequest> {
        self.viewer
            .pop_front()
            .or_else(|| self.prefetch.pop_front())
    }
}

#[derive(Default)]
struct SharedPreviewQueue {
    state: Mutex<PreviewRequestQueue>,
    wake: Condvar,
    shutdown: AtomicBool,
}

impl SharedPreviewQueue {
    fn push(&self, request: PreviewRequest) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.push(request);
        self.wake.notify_one();
    }

    fn pop_blocking(&self) -> Option<PreviewRequest> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(request) = state.pop() {
                return Some(request);
            }
            if self.shutdown.load(Ordering::Relaxed) {
                return None;
            }
            state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn close(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.wake.notify_all();
    }
}

/// A cheap-to-clone handle for submitting preview decode requests to the worker pool.
#[derive(Clone)]
pub struct PreviewHandle {
    queue: Arc<SharedPreviewQueue>,
    latest_viewer_gen: Arc<AtomicU64>,
    latest_prefetch_gen: Arc<AtomicU64>,
}

impl PreviewHandle {
    pub fn request_viewer(&self, path: PathBuf, gen: u64) {
        self.latest_viewer_gen.store(gen, Ordering::Relaxed);
        self.queue.push(PreviewRequest {
            path,
            gen,
            role: PreviewRequestRole::Viewer,
            prefetch: None,
        });
    }

    pub fn request_prefetch(&self, path: PathBuf, gen: u64, prefetch: PrefetchRequestContext) {
        self.latest_prefetch_gen.store(gen, Ordering::Relaxed);
        self.queue.push(PreviewRequest {
            path,
            gen,
            role: PreviewRequestRole::Prefetch,
            prefetch: Some(prefetch),
        });
    }

    fn current_gen_for(&self, role: PreviewRequestRole) -> u64 {
        match role {
            PreviewRequestRole::Viewer => self.latest_viewer_gen.load(Ordering::Relaxed),
            PreviewRequestRole::Prefetch => self.latest_prefetch_gen.load(Ordering::Relaxed),
        }
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
    pub fn spawn() -> (Self, Receiver<PreviewResult>, Receiver<PreviewResult>) {
        let (viewer_result_tx, viewer_result_rx) = async_channel::unbounded::<PreviewResult>();
        let (prefetch_result_tx, prefetch_result_rx) = async_channel::unbounded::<PreviewResult>();
        let queue = Arc::new(SharedPreviewQueue::default());
        let latest_viewer_gen = Arc::new(AtomicU64::new(0));
        let latest_prefetch_gen = Arc::new(AtomicU64::new(0));

        for _ in 0..PREVIEW_WORKER_COUNT {
            let queue = queue.clone();
            let handle = PreviewHandle {
                queue: queue.clone(),
                latest_viewer_gen: latest_viewer_gen.clone(),
                latest_prefetch_gen: latest_prefetch_gen.clone(),
            };
            let viewer_tx = viewer_result_tx.clone();
            let prefetch_tx = prefetch_result_tx.clone();
            std::thread::spawn(move || loop {
                let Some(req) = queue.pop_blocking() else {
                    break;
                };
                let current_gen = handle.current_gen_for(req.role);
                if req.gen != current_gen {
                    crate::bench_event!(
                        "preview.skip_stale",
                        serde_json::json!({
                            "path": req.path.display().to_string(),
                            "role": preview_role_label(req.role),
                            "request_gen": req.gen,
                            "current_gen": current_gen,
                        }),
                    );
                    continue;
                }
                let started = Instant::now();
                let mode = match req.role {
                    PreviewRequestRole::Viewer => PreviewDecodeMode::Viewer,
                    PreviewRequestRole::Prefetch => PreviewDecodeMode::Prefetch,
                };
                let image = decode_preview(&req.path, mode);
                let current_gen = handle.current_gen_for(req.role);
                if req.gen != current_gen {
                    crate::bench_event!(
                        "preview.stale_result",
                        serde_json::json!({
                            "path": req.path.display().to_string(),
                            "role": preview_role_label(req.role),
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
                        "role": preview_role_label(req.role),
                        "gen": req.gen,
                        "success": image.is_ok(),
                        "duration_ms": crate::bench::duration_ms(started),
                    }),
                );
                let result = PreviewResult {
                    path: req.path,
                    gen: req.gen,
                    role: req.role,
                    prefetch: req.prefetch,
                    image,
                };
                let tx = match result.role {
                    PreviewRequestRole::Viewer => &viewer_tx,
                    PreviewRequestRole::Prefetch => &prefetch_tx,
                };
                let _ = tx.send_blocking(result);
            });
        }

        let handle = PreviewHandle {
            queue,
            latest_viewer_gen,
            latest_prefetch_gen,
        };
        (Self { handle }, viewer_result_rx, prefetch_result_rx)
    }

    /// Return a cloneable request handle to pass to the viewer.
    pub fn handle(&self) -> PreviewHandle {
        self.handle.clone()
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        self.handle.queue.close();
    }
}

fn preview_role_label(role: PreviewRequestRole) -> &'static str {
    match role {
        PreviewRequestRole::Viewer => "viewer",
        PreviewRequestRole::Prefetch => "prefetch",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_requests_outrank_pending_prefetch() {
        let mut queue = PreviewRequestQueue::default();
        queue.push(PreviewRequest {
            path: PathBuf::from("/tmp/prefetch.webp"),
            gen: 1,
            role: PreviewRequestRole::Prefetch,
            prefetch: Some(PrefetchRequestContext {
                index: 1,
                direction: 1,
                distance: 1,
            }),
        });
        queue.push(PreviewRequest {
            path: PathBuf::from("/tmp/viewer.webp"),
            gen: 2,
            role: PreviewRequestRole::Viewer,
            prefetch: None,
        });

        let first = queue.pop().expect("request");
        assert_eq!(first.role, PreviewRequestRole::Viewer);
        assert_eq!(first.path, PathBuf::from("/tmp/viewer.webp"));
    }

    #[test]
    fn viewer_staleness_is_scoped_separately_from_prefetch() {
        let handle = PreviewHandle {
            queue: Arc::new(SharedPreviewQueue::default()),
            latest_viewer_gen: Arc::new(AtomicU64::new(7)),
            latest_prefetch_gen: Arc::new(AtomicU64::new(21)),
        };

        assert_eq!(handle.current_gen_for(PreviewRequestRole::Viewer), 7);
        assert_eq!(handle.current_gen_for(PreviewRequestRole::Prefetch), 21);
    }
}
