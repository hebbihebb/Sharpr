use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_channel::{Receiver, Sender};

/// Message sent from the UI to the worker pool.
///
/// `gen` is the folder-switch generation at the time the request was queued.
/// Workers compare it against the shared atomic before starting expensive
/// decode work — stale requests (different generation) are skipped immediately.
pub enum WorkerRequest {
    Thumbnail { path: PathBuf, gen: u64 },
    Hash { path: PathBuf, gen: u64 },
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

/// Message sent back from a worker thread after computing a perceptual hash.
pub struct HashResult {
    pub path: PathBuf,
    pub hash: u64,
}

/// Manages a pool of background threads that decode and resize images.
///
/// The caller owns the `result_rx` receiver and polls it via
/// `glib::MainContext::spawn_local`.
pub struct ThumbnailWorker {
    request_tx: Sender<WorkerRequest>,
    /// Monotonically increasing folder-switch counter shared with all workers.
    /// Bump this (via `bump_generation`) when the folder changes; workers skip
    /// any pending requests whose `gen` no longer matches.
    generation: Arc<AtomicU64>,
}

/// Target height for generated thumbnails (in pixels).
const THUMB_HEIGHT: u32 = 160;

impl ThumbnailWorker {
    /// Spawn `thread_count` background workers.
    /// Returns the worker handle and the receivers for thumbnail and hash results.
    pub fn spawn(thread_count: usize) -> (Self, Receiver<ThumbnailResult>, Receiver<HashResult>) {
        let (request_tx, request_rx) = async_channel::unbounded::<WorkerRequest>();
        let (result_tx, result_rx) = async_channel::unbounded::<ThumbnailResult>();
        let (hash_result_tx, hash_result_rx) = async_channel::unbounded::<HashResult>();
        let generation = Arc::new(AtomicU64::new(0));

        for _ in 0..thread_count {
            let request_rx = request_rx.clone();
            let result_tx = result_tx.clone();
            let hash_result_tx = hash_result_tx.clone();
            let gen_arc = generation.clone();
            std::thread::spawn(move || {
                loop {
                    match request_rx.recv_blocking() {
                        Ok(WorkerRequest::Thumbnail { path, gen }) => {
                            // Skip stale requests immediately — no decode needed.
                            if gen != gen_arc.load(Ordering::Relaxed) {
                                continue;
                            }
                            if let Some(result) = generate_thumbnail(&path) {
                                let _ = result_tx.send_blocking(result);
                            }
                        }
                        Ok(WorkerRequest::Hash { path, gen }) => {
                            if gen != gen_arc.load(Ordering::Relaxed) {
                                continue;
                            }
                            if let Some(hash) = compute_hash(&path) {
                                let _ = hash_result_tx.send_blocking(HashResult { path, hash });
                            }
                        }
                        Ok(WorkerRequest::Shutdown) | Err(_) => break,
                    }
                }
            });
        }

        (
            Self {
                request_tx,
                generation,
            },
            result_rx,
            hash_result_rx,
        )
    }

    /// Increment the generation counter (call on every folder switch).
    /// Returns the new generation value.
    pub fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Current generation value — embed in `WorkerRequest::Thumbnail`.
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Return a cloned sender so other components can submit requests directly.
    pub fn sender(&self) -> Sender<WorkerRequest> {
        self.request_tx.clone()
    }

    /// Return a clone of the generation Arc for use in the filmstrip.
    pub fn generation_arc(&self) -> Arc<AtomicU64> {
        self.generation.clone()
    }
}

impl Drop for ThumbnailWorker {
    fn drop(&mut self) {
        // Signal all threads to stop.
        for _ in 0..4 {
            let _ = self.request_tx.try_send(WorkerRequest::Shutdown);
        }
    }
}

// ---------------------------------------------------------------------------
// Thumbnail generation (runs on worker threads)
// ---------------------------------------------------------------------------

fn generate_thumbnail(path: &Path) -> Option<ThumbnailResult> {
    // Fast path: return cached PNG if it exists and is fresh.
    if let Some(cached) = load_cached_thumbnail(path) {
        return Some(cached);
    }

    // Try rexiv2 embedded preview before full decode.
    if let Ok(meta) = rexiv2::Metadata::new_from_path(path) {
        if let Ok(previews) = meta.get_preview_images() {
            for preview in previews {
                if let Ok(data) = preview.get_data() {
                    if let Ok(img) = image::load_from_memory(&data) {
                        return build_thumbnail_and_cache(path, img);
                    }
                }
            }
        }
    }

    let file = std::fs::File::open(path).ok()?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    let img = apply_exif_orientation(img, path);
    build_thumbnail_and_cache(path, img)
}

fn build_thumbnail_and_cache(path: &Path, img: image::DynamicImage) -> Option<ThumbnailResult> {
    use image::imageops::{self, FilterType};

    let orig_w = img.width();
    let orig_h = img.height();
    if orig_h == 0 {
        return None;
    }

    let thumb_h = THUMB_HEIGHT;
    let thumb_w = ((orig_w as f64 / orig_h as f64) * thumb_h as f64).round() as u32;
    let thumb_w = thumb_w.max(1);

    let resized = imageops::resize(&img.into_rgba8(), thumb_w, thumb_h, FilterType::Lanczos3);
    let rgba_bytes = resized.into_raw();

    write_thumbnail_cache(path, &rgba_bytes, thumb_w, thumb_h);

    Some(ThumbnailResult {
        path: path.to_path_buf(),
        rgba_bytes,
        width: thumb_w,
        height: thumb_h,
    })
}

fn load_cached_thumbnail(path: &Path) -> Option<ThumbnailResult> {
    let cache_path = thumbnail_cache_path(path)?;
    if !cache_path.exists() {
        return None;
    }
    let img = image::open(&cache_path).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(ThumbnailResult {
        path: path.to_path_buf(),
        rgba_bytes: img.into_raw(),
        width,
        height,
    })
}

fn write_thumbnail_cache(path: &Path, rgba_bytes: &[u8], width: u32, height: u32) {
    let Some(cache_path) = thumbnail_cache_path(path) else {
        return;
    };
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let format = image::ImageFormat::Png;
    let _ = image::save_buffer_with_format(
        &cache_path,
        rgba_bytes,
        width,
        height,
        image::ColorType::Rgba8,
        format,
    );
}

fn thumbnail_cache_path(source_path: &Path) -> Option<PathBuf> {
    super::cache::thumbnail_cache_path(source_path)
}

fn apply_exif_orientation(img: image::DynamicImage, path: &Path) -> image::DynamicImage {
    use image::imageops;
    use image::DynamicImage;

    let orientation: u32 = rexiv2::Metadata::new_from_path(path)
        .ok()
        .and_then(|meta| meta.get_tag_string("Exif.Image.Orientation").ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1);

    match orientation {
        1 => img,
        2 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&img.into_rgba8())),
        3 => DynamicImage::ImageRgba8(imageops::rotate180(&img.into_rgba8())),
        4 => DynamicImage::ImageRgba8(imageops::flip_vertical(&img.into_rgba8())),
        5 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&imageops::rotate90(
            &img.into_rgba8(),
        ))),
        6 => DynamicImage::ImageRgba8(imageops::rotate90(&img.into_rgba8())),
        7 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&imageops::rotate270(
            &img.into_rgba8(),
        ))),
        8 => DynamicImage::ImageRgba8(imageops::rotate270(&img.into_rgba8())),
        _ => img,
    }
}

fn compute_hash(path: &Path) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    Some(crate::duplicates::phash::dhash(&img))
}
