use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_channel::{Receiver, Sender};

use crate::metadata::ImageMetadata;

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
