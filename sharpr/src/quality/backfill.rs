use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;

use crate::thumbnails::worker::SharpnessResult;

/// Idle background worker that scores images whose thumbnails are already
/// cached but whose sharpness hasn't been computed yet.
///
/// Images get scored naturally as thumbnails are decoded by the main worker
/// pools.  This backfill handles images that are in the library but haven't
/// been scrolled past yet, and also acts as a migration pass on first launch.
///
/// Throttled to ~12 images/second (80 ms sleep between each) to stay
/// invisible to the user.
pub struct SharpnessBackfill {
    queue_tx: async_channel::Sender<PathBuf>,
}

impl SharpnessBackfill {
    /// Spawn the backfill thread.  Results are sent to `result_tx`, which
    /// should be the same sender that `ThumbnailWorker` uses so the window
    /// only needs one receiver.
    pub fn spawn(
        db: Arc<crate::tags::TagDatabase>,
        result_tx: Sender<SharpnessResult>,
    ) -> Self {
        let (queue_tx, queue_rx) = async_channel::unbounded::<PathBuf>();

        std::thread::Builder::new()
            .name("sharpr-sharpness-backfill".into())
            .spawn(move || {
                loop {
                    match queue_rx.recv_blocking() {
                        Ok(path) => {
                            if db.get_sharpness(&path).is_some() {
                                continue;
                            }
                            if let Some(score) = score_from_cache(&path) {
                                let mtime =
                                    crate::tags::db::file_mtime_secs(&path).unwrap_or(0);
                                db.upsert_sharpness(&path, score, mtime);
                                let _ = result_tx.send_blocking(SharpnessResult {
                                    path,
                                    score,
                                });
                            }
                            std::thread::sleep(Duration::from_millis(80));
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();

        Self { queue_tx }
    }

    /// Enqueue a batch of paths to score.  Already-scored paths are skipped
    /// cheaply by the thread, so it is safe to enqueue the full library.
    pub fn enqueue(&self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            let _ = self.queue_tx.try_send(path);
        }
    }
}

/// Load thumbnail pixels from disk/system cache and compute Laplacian variance.
/// Returns `None` when no cached thumbnail exists (image will be scored later
/// when the user scrolls to it and the main workers decode it).
fn score_from_cache(path: &std::path::Path) -> Option<f64> {
    let (bytes, w, h) = crate::thumbnails::worker::load_cached_rgba(path)?;
    Some(crate::quality::blur::laplacian_variance(&bytes, w, h))
}
