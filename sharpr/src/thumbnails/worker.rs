use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_channel::{Receiver, Sender};
use gtk4::gio;
use gtk4::prelude::*;

use crate::metadata::orientation::{
    apply_exif_orientation, apply_exif_orientation_value, extract_exif_data,
};

/// Sent back when a sharpness score is computed for an image.
pub struct SharpnessResult {
    pub path: PathBuf,
    /// Raw Laplacian variance; pass through `blur::normalize_sharpness` for display.
    pub score: f64,
}

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
    pub source: &'static str,
    pub worker_ms: u128,
}

/// Message sent back from a worker thread after computing a perceptual hash.
pub struct HashResult {
    pub path: PathBuf,
    pub hash: u64,
}

/// Manages two pools of background threads: a high-priority visible pool and a
/// low-priority preload pool. Both share the same result receivers.
///
/// The caller owns the `result_rx` receiver and polls it via
/// `glib::MainContext::spawn_local`.
pub struct ThumbnailWorker {
    visible_tx: Sender<WorkerRequest>,
    preload_tx: Sender<WorkerRequest>,
    /// Monotonically increasing folder-switch counter shared with all workers.
    /// Bump this (via `bump_generation`) when the folder changes; workers skip
    /// any pending requests whose `gen` no longer matches.
    generation: Arc<AtomicU64>,
    /// Paths currently queued or running for thumbnail generation.
    pending_paths: Arc<Mutex<HashSet<PathBuf>>>,
    /// Number of visible-channel workers, used for clean shutdown.
    visible_worker_count: usize,
    preload_worker_count: usize,
    /// Sender side of the sharpness channel — hand to the backfill worker too.
    sharpness_tx: Sender<SharpnessResult>,
}

/// Target height for generated thumbnails (in pixels).
const THUMB_HEIGHT: u32 = 160;

impl ThumbnailWorker {
    /// Spawn background workers split into a visible pool (`thread_count - 1`
    /// threads) and a preload pool (1 thread).
    /// Returns the worker handle and the receivers for thumbnail, hash, and sharpness results.
    pub fn spawn(
        thread_count: usize,
        db: Option<Arc<crate::tags::TagDatabase>>,
    ) -> (
        Self,
        Receiver<ThumbnailResult>,
        Receiver<HashResult>,
        Receiver<SharpnessResult>,
    ) {
        let preload_workers = 4usize;
        let visible_workers = thread_count.saturating_sub(preload_workers).max(1);

        let (visible_tx, visible_rx) = async_channel::unbounded::<WorkerRequest>();
        let (preload_tx, preload_rx) = async_channel::unbounded::<WorkerRequest>();
        let (result_tx, result_rx) = async_channel::unbounded::<ThumbnailResult>();
        let (hash_result_tx, hash_result_rx) = async_channel::unbounded::<HashResult>();
        let (sharpness_tx, sharpness_rx) = async_channel::unbounded::<SharpnessResult>();
        let generation = Arc::new(AtomicU64::new(0));
        let pending_paths = Arc::new(Mutex::new(HashSet::new()));

        // Visible workers — service in-viewport tile requests at normal priority.
        for _ in 0..visible_workers {
            let request_rx = visible_rx.clone();
            let request_tx = visible_tx.clone();
            let result_tx = result_tx.clone();
            let hash_result_tx = hash_result_tx.clone();
            let sharpness_tx = sharpness_tx.clone();
            let db = db.clone();
            let gen_arc = generation.clone();
            let pending_paths = pending_paths.clone();
            std::thread::spawn(move || {
                loop {
                    match request_rx.recv_blocking() {
                        Ok(WorkerRequest::Thumbnail { path, gen }) => {
                            // Skip stale requests immediately — no decode needed.
                            if gen != gen_arc.load(Ordering::Relaxed) {
                                crate::bench_event!(
                                    "thumbnail.skip_stale",
                                    serde_json::json!({
                                        "path": path.display().to_string(),
                                        "pool": "visible",
                                        "request_gen": gen,
                                        "current_gen": gen_arc.load(Ordering::Relaxed),
                                    }),
                                );
                                if let Ok(mut pending) = pending_paths.lock() {
                                    pending.remove(&path);
                                }
                                continue;
                            }
                            crate::bench_event!(
                                "thumbnail.start",
                                serde_json::json!({
                                    "path": path.display().to_string(),
                                    "pool": "visible",
                                    "gen": gen,
                                }),
                            );
                            if let Some(result) = generate_thumbnail(&path) {
                                crate::bench_event!(
                                    "thumbnail.finish",
                                    serde_json::json!({
                                        "path": path.display().to_string(),
                                        "pool": "visible",
                                        "gen": gen,
                                        "source": result.source,
                                        "width": result.width,
                                        "height": result.height,
                                        "duration_ms": result.worker_ms,
                                    }),
                                );
                                let sharpness = maybe_score_sharpness(&path, &result, &db);
                                let _ = result_tx.send_blocking(result);
                                if let Some(sr) = sharpness {
                                    let _ = sharpness_tx.send_blocking(sr);
                                }
                            } else {
                                crate::bench_event!(
                                    "thumbnail.fail",
                                    serde_json::json!({
                                        "path": path.display().to_string(),
                                        "pool": "visible",
                                        "gen": gen,
                                    }),
                                );
                            }

                            // Chain hashing request to the same worker pool.
                            // This ensures hashing happens even if the thumbnail is already in cache.
                            let _ = request_tx.send_blocking(WorkerRequest::Hash {
                                path: path.clone(),
                                gen,
                            });

                            if let Ok(mut pending) = pending_paths.lock() {
                                pending.remove(&path);
                            }
                        }
                        Ok(WorkerRequest::Hash { path, gen }) => {
                            if gen != gen_arc.load(Ordering::Relaxed) {
                                continue;
                            }
                            let started = Instant::now();
                            if let Some(hash) = compute_hash(&path) {
                                crate::bench_event!(
                                    "hash.finish",
                                    serde_json::json!({
                                        "path": path.display().to_string(),
                                        "pool": "visible",
                                        "gen": gen,
                                        "duration_ms": crate::bench::duration_ms(started),
                                    }),
                                );
                                let _ = hash_result_tx.send_blocking(HashResult { path, hash });
                            } else {
                                crate::bench_event!(
                                    "hash.fail",
                                    serde_json::json!({
                                        "path": path.display().to_string(),
                                        "pool": "visible",
                                        "gen": gen,
                                        "duration_ms": crate::bench::duration_ms(started),
                                    }),
                                );
                            }
                        }
                        Ok(WorkerRequest::Shutdown) | Err(_) => break,
                    }
                }
            });
        }

        // Preload workers — generate off-screen thumbnails at background priority.
        for _ in 0..preload_workers {
            let request_rx = preload_rx.clone();
            let request_tx = preload_tx.clone();
            let result_tx = result_tx.clone();
            let hash_result_tx = hash_result_tx.clone();
            let sharpness_tx = sharpness_tx.clone();
            let db = db.clone();
            let gen_arc = generation.clone();
            let pending_paths = pending_paths.clone();
            std::thread::spawn(move || loop {
                match request_rx.recv_blocking() {
                    Ok(WorkerRequest::Thumbnail { path, gen }) => {
                        if gen != gen_arc.load(Ordering::Relaxed) {
                            crate::bench_event!(
                                "thumbnail.skip_stale",
                                serde_json::json!({
                                    "path": path.display().to_string(),
                                    "pool": "preload",
                                    "request_gen": gen,
                                    "current_gen": gen_arc.load(Ordering::Relaxed),
                                }),
                            );
                            if let Ok(mut pending) = pending_paths.lock() {
                                pending.remove(&path);
                            }
                            continue;
                        }
                        crate::bench_event!(
                            "thumbnail.start",
                            serde_json::json!({
                                "path": path.display().to_string(),
                                "pool": "preload",
                                "gen": gen,
                            }),
                        );
                        if let Some(result) = generate_thumbnail(&path) {
                            crate::bench_event!(
                                "thumbnail.finish",
                                serde_json::json!({
                                    "path": path.display().to_string(),
                                    "pool": "preload",
                                    "gen": gen,
                                    "source": result.source,
                                    "width": result.width,
                                    "height": result.height,
                                    "duration_ms": result.worker_ms,
                                }),
                            );
                            let sharpness = maybe_score_sharpness(&path, &result, &db);
                            let _ = result_tx.send_blocking(result);
                            if let Some(sr) = sharpness {
                                let _ = sharpness_tx.send_blocking(sr);
                            }
                        } else {
                            crate::bench_event!(
                                "thumbnail.fail",
                                serde_json::json!({
                                    "path": path.display().to_string(),
                                    "pool": "preload",
                                    "gen": gen,
                                }),
                            );
                        }

                        // Hashing is low priority, so we chain it here.
                        let _ = request_tx.send_blocking(WorkerRequest::Hash {
                            path: path.clone(),
                            gen,
                        });

                        if let Ok(mut pending) = pending_paths.lock() {
                            pending.remove(&path);
                        }
                    }
                    Ok(WorkerRequest::Hash { path, gen }) => {
                        if gen != gen_arc.load(Ordering::Relaxed) {
                            continue;
                        }
                        let started = Instant::now();
                        if let Some(hash) = compute_hash(&path) {
                            crate::bench_event!(
                                "hash.finish",
                                serde_json::json!({
                                    "path": path.display().to_string(),
                                    "pool": "preload",
                                    "gen": gen,
                                    "duration_ms": crate::bench::duration_ms(started),
                                }),
                            );
                            let _ = hash_result_tx.send_blocking(HashResult { path, hash });
                        } else {
                            crate::bench_event!(
                                "hash.fail",
                                serde_json::json!({
                                    "path": path.display().to_string(),
                                    "pool": "preload",
                                    "gen": gen,
                                    "duration_ms": crate::bench::duration_ms(started),
                                }),
                            );
                        }
                    }
                    Ok(WorkerRequest::Shutdown) | Err(_) => break,
                }
            });
        }

        (
            Self {
                visible_tx,
                preload_tx,
                generation,
                pending_paths,
                visible_worker_count: visible_workers,
                preload_worker_count: preload_workers,
                sharpness_tx,
            },
            result_rx,
            hash_result_rx,
            sharpness_rx,
        )
    }

    /// Increment the generation counter (call on every folder switch).
    /// Clears the in-flight dedup set and returns the new generation value.
    pub fn bump_generation(&self) -> u64 {
        if let Ok(mut pending) = self.pending_paths.lock() {
            pending.clear();
        }
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Current generation value — embed in `WorkerRequest::Thumbnail`.
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Sender for in-viewport (high-priority) thumbnail requests.
    pub fn visible_sender(&self) -> Sender<WorkerRequest> {
        self.visible_tx.clone()
    }

    /// Sender for off-screen pre-generation (low-priority) requests.
    pub fn preload_sender(&self) -> Sender<WorkerRequest> {
        self.preload_tx.clone()
    }

    /// Shared in-flight thumbnail path set for enqueue deduplication.
    pub fn pending_set(&self) -> Arc<Mutex<HashSet<PathBuf>>> {
        self.pending_paths.clone()
    }

    /// Return a clone of the generation Arc for use in the filmstrip.
    pub fn generation_arc(&self) -> Arc<AtomicU64> {
        self.generation.clone()
    }

    /// Clone of the sharpness sender — hand this to the backfill worker so
    /// results flow through the same channel.
    pub fn sharpness_sender(&self) -> Sender<SharpnessResult> {
        self.sharpness_tx.clone()
    }
}

impl Drop for ThumbnailWorker {
    fn drop(&mut self) {
        // Signal all threads to stop — one Shutdown per spawned thread.
        for _ in 0..self.visible_worker_count {
            let _ = self.visible_tx.try_send(WorkerRequest::Shutdown);
        }
        for _ in 0..self.preload_worker_count {
            let _ = self.preload_tx.try_send(WorkerRequest::Shutdown);
        }
    }
}

// ---------------------------------------------------------------------------
// Thumbnail generation (runs on worker threads)
// ---------------------------------------------------------------------------

fn generate_thumbnail(path: &Path) -> Option<ThumbnailResult> {
    let started = Instant::now();
    // Try system-wide thumbnails first (GNOME/Freedesktop standard) as they
    // are likely already generated and high quality.
    if let Some(mut system_cached) = load_system_thumbnail(path) {
        system_cached.worker_ms = crate::bench::duration_ms(started);
        return Some(system_cached);
    }

    // Fall back to Sharpr's own disk cache.
    if let Some(mut cached) = load_cached_thumbnail(path) {
        cached.worker_ms = crate::bench::duration_ms(started);
        return Some(cached);
    }

    let is_jpeg = path
        .extension()
        .map(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "jpg" | "jpeg"))
        .unwrap_or(false);

    if is_jpeg {
        let (thumb_opt, orientation) = extract_exif_data(path, THUMB_HEIGHT);
        if let Some(img) = thumb_opt {
            let img = apply_exif_orientation_value(img, orientation);
            let mut result = build_thumbnail_and_cache(path, img, "embedded_jpeg")?;
            result.worker_ms = crate::bench::duration_ms(started);
            return Some(result);
        }

        let img = decode_jpeg_scaled(path).or_else(|| decode_with_image_crate(path))?;
        let img = apply_exif_orientation_value(img, orientation);
        let mut result = build_thumbnail_and_cache(path, img, "decoded_jpeg")?;
        result.worker_ms = crate::bench::duration_ms(started);
        return Some(result);
    }

    if is_webp_path(path) {
        let img = decode_webp_scaled(path, THUMB_HEIGHT)?;
        let img = apply_exif_orientation(img, path);
        let mut result = build_thumbnail_and_cache(path, img, "decoded_webp")?;
        result.worker_ms = crate::bench::duration_ms(started);
        return Some(result);
    }

    if crate::jxl::is_jxl_path(path) {
        let img = crate::jxl::decode_path(path).ok()?;
        let img = apply_exif_orientation(img, path);
        let (target_width, target_height) =
            choose_thumbnail_webp_dimensions(img.width(), img.height(), THUMB_HEIGHT);
        let img = if target_width != img.width() || target_height != img.height() {
            img.resize(
                target_width,
                target_height,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            img
        };
        let mut result = build_thumbnail_and_cache(path, img, "decoded_jxl")?;
        result.worker_ms = crate::bench::duration_ms(started);
        return Some(result);
    }

    let img = decode_with_image_crate(path)?;
    let img = apply_exif_orientation(img, path);
    let mut result = build_thumbnail_and_cache(path, img, "decoded_image")?;
    result.worker_ms = crate::bench::duration_ms(started);
    Some(result)
}

fn decode_jpeg_scaled(path: &Path) -> Option<image::DynamicImage> {
    let file = std::fs::File::open(path).ok()?;
    // Safety: the file is read-only and not modified while the map is live.
    // The map is dropped (unmapped) at the end of this function.
    let mmap = unsafe { memmap2::MmapOptions::new().map(&file).ok()? };
    let data: &[u8] = &mmap;
    let mut decompressor = turbojpeg::Decompressor::new().ok()?;
    let header = decompressor.read_header(data).ok()?;
    if header.is_lossless {
        return None;
    }

    let short_side = header.width.min(header.height);
    let factor = turbojpeg::Decompressor::supported_scaling_factors()
        .into_iter()
        .filter(|factor| factor.scale(short_side) >= THUMB_HEIGHT as usize)
        .min_by_key(|factor| factor.scale(short_side))
        .unwrap_or(turbojpeg::ScalingFactor::ONE);

    decompressor.set_scaling_factor(factor).ok()?;
    let scaled = header.scaled(factor);
    let pitch = scaled.width * turbojpeg::PixelFormat::RGBA.size();
    let mut image = turbojpeg::Image {
        pixels: vec![0; pitch * scaled.height],
        width: scaled.width,
        pitch,
        height: scaled.height,
        format: turbojpeg::PixelFormat::RGBA,
    };

    decompressor.decompress(data, image.as_deref_mut()).ok()?;

    let rgba = image::RgbaImage::from_raw(scaled.width as u32, scaled.height as u32, image.pixels)?;
    Some(image::DynamicImage::ImageRgba8(rgba))
}

fn is_webp_path(path: &Path) -> bool {
    let by_extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "webp"))
        .unwrap_or(false);
    if by_extension {
        return true;
    }

    let mut file = std::fs::File::open(path).ok();
    let Some(file) = file.as_mut() else {
        return false;
    };
    let mut magic = [0u8; 12];
    std::io::Read::read_exact(file, &mut magic).is_ok()
        && &magic[0..4] == b"RIFF"
        && &magic[8..12] == b"WEBP"
}

fn decode_webp_scaled(path: &Path, min_short_edge: u32) -> Option<image::DynamicImage> {
    let webp_data = std::fs::read(path).ok()?;
    let mut config = libwebp_sys::WebPDecoderConfig::new().ok()?;
    let status = unsafe {
        libwebp_sys::WebPGetFeatures(webp_data.as_ptr(), webp_data.len(), &mut config.input)
    };
    if status != libwebp_sys::VP8StatusCode::VP8_STATUS_OK {
        return None;
    }
    if config.input.has_animation != 0 {
        return None;
    }

    let src_width = u32::try_from(config.input.width).ok()?;
    let src_height = u32::try_from(config.input.height).ok()?;
    let (target_width, target_height) =
        choose_thumbnail_webp_dimensions(src_width, src_height, min_short_edge);
    let stride = usize::try_from(target_width).ok()?.checked_mul(4)?;
    let mut rgba = vec![0u8; stride.checked_mul(usize::try_from(target_height).ok()?)?];

    config.options.use_threads = 1;
    let use_scaling = target_width != src_width || target_height != src_height;
    config.options.use_scaling = i32::from(use_scaling);
    config.options.scaled_width = i32::try_from(target_width).ok()?;
    config.options.scaled_height = i32::try_from(target_height).ok()?;
    config.output.colorspace = libwebp_sys::WEBP_CSP_MODE::MODE_RGBA;
    config.output.width = i32::try_from(target_width).ok()?;
    config.output.height = i32::try_from(target_height).ok()?;
    config.output.is_external_memory = 1;
    config.output.u.RGBA.rgba = rgba.as_mut_ptr();
    config.output.u.RGBA.stride = i32::try_from(stride).ok()?;
    config.output.u.RGBA.size = rgba.len();

    let status =
        unsafe { libwebp_sys::WebPDecode(webp_data.as_ptr(), webp_data.len(), &mut config) };
    if status != libwebp_sys::VP8StatusCode::VP8_STATUS_OK {
        return None;
    }

    let rgba = image::RgbaImage::from_raw(target_width, target_height, rgba)?;
    Some(image::DynamicImage::ImageRgba8(rgba))
}

fn decode_with_image_crate(path: &Path) -> Option<image::DynamicImage> {
    if crate::jxl::is_jxl_path(path) {
        return crate::jxl::decode_path(path).ok();
    }

    let file = std::fs::File::open(path).ok()?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    reader.decode().ok()
}

fn choose_thumbnail_webp_dimensions(width: u32, height: u32, min_short_edge: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (width, height);
    }

    let short_edge = width.min(height);
    if short_edge <= min_short_edge {
        return (width, height);
    }

    let scale = min_short_edge as f64 / short_edge as f64;
    let scaled_width = ((width as f64) * scale).round().max(1.0) as u32;
    let scaled_height = ((height as f64) * scale).round().max(1.0) as u32;
    (scaled_width, scaled_height)
}

fn build_thumbnail_and_cache(
    path: &Path,
    img: image::DynamicImage,
    source: &'static str,
) -> Option<ThumbnailResult> {
    use image::imageops::{self, FilterType};

    let orig_w = img.width();
    let orig_h = img.height();
    if orig_h == 0 {
        return None;
    }

    let thumb_h = THUMB_HEIGHT;
    let thumb_w = ((orig_w as f64 / orig_h as f64) * thumb_h as f64).round() as u32;
    let thumb_w = thumb_w.max(1);

    let resized = imageops::resize(&img.into_rgba8(), thumb_w, thumb_h, FilterType::Triangle);
    let rgba_bytes = resized.into_raw();

    write_thumbnail_cache(path, &rgba_bytes, thumb_w, thumb_h);

    Some(ThumbnailResult {
        path: path.to_path_buf(),
        rgba_bytes,
        width: thumb_w,
        height: thumb_h,
        source,
        worker_ms: 0,
    })
}

fn load_cached_thumbnail(path: &Path) -> Option<ThumbnailResult> {
    let cache_path = thumbnail_cache_path(path)?;
    if !cache_path.exists() {
        return None;
    }
    let img = image::open(&cache_path).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    if height != THUMB_HEIGHT {
        return None;
    }
    Some(ThumbnailResult {
        path: path.to_path_buf(),
        rgba_bytes: img.into_raw(),
        width,
        height,
        source: "sharpr_disk",
        worker_ms: 0,
    })
}

fn load_system_thumbnail(path: &Path) -> Option<ThumbnailResult> {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let file = gio::File::for_path(&abs_path);

    let info = file
        .query_info(
            "thumbnail::path,thumbnail::is-valid",
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .ok()?;

    // Treat absent `thumbnail::is-valid` as "not explicitly invalid": some GIO
    // backends omit the attribute even for fresh thumbnails.  Only skip if it is
    // explicitly "FALSE".  A missing/corrupt file is handled by `image::open()`.
    let explicitly_invalid = info
        .attribute_as_string("thumbnail::is-valid")
        .map(|s| s.as_str() == "FALSE")
        .unwrap_or(false);

    if !explicitly_invalid {
        if let Some(thumb_path) = info.attribute_as_string("thumbnail::path") {
            let thumb_path = PathBuf::from(thumb_path.as_str());
            if let Ok(img) = image::open(&thumb_path) {
                let mut img = img.into_rgba8();
                let (mut width, mut height) = img.dimensions();

                if height > THUMB_HEIGHT * 2 {
                    let new_w =
                        ((width as f64 / height as f64) * THUMB_HEIGHT as f64).round() as u32;
                    img = image::imageops::resize(
                        &img,
                        new_w.max(1),
                        THUMB_HEIGHT,
                        image::imageops::FilterType::Triangle,
                    );
                    let (w, h) = img.dimensions();
                    width = w;
                    height = h;
                }

                return Some(ThumbnailResult {
                    path: path.to_path_buf(),
                    rgba_bytes: img.into_raw(),
                    width,
                    height,
                    source: "system",
                    worker_ms: 0,
                });
            }
        }
    }

    None
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

/// Load cached thumbnail pixels (Sharpr disk cache or system GNOME cache).
/// Used by both the backfill worker and `compute_hash`.
pub(crate) fn load_cached_rgba(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    load_cached_thumbnail(path)
        .or_else(|| load_system_thumbnail(path))
        .map(|t| (t.rgba_bytes, t.width, t.height))
}

/// Compute and persist a sharpness score if the image is not already scored.
/// Returns `Some(SharpnessResult)` when a new score was calculated.
fn maybe_score_sharpness(
    path: &Path,
    result: &ThumbnailResult,
    db: &Option<Arc<crate::tags::TagDatabase>>,
) -> Option<SharpnessResult> {
    let db = db.as_ref()?;
    if db.get_sharpness(path).is_some() {
        return None;
    }
    let score =
        crate::quality::blur::laplacian_variance(&result.rgba_bytes, result.width, result.height);
    let mtime = crate::tags::db::file_mtime_secs(path).unwrap_or(0);
    db.upsert_sharpness(path, score, mtime);
    Some(SharpnessResult {
        path: path.to_path_buf(),
        score,
    })
}

fn compute_hash(path: &Path) -> Option<u64> {
    if let Some((bytes, w, h)) = load_cached_rgba(path) {
        if let Some(rgba) = image::RgbaImage::from_raw(w, h, bytes) {
            let img = image::DynamicImage::ImageRgba8(rgba);
            return Some(crate::duplicates::phash::dhash(&img));
        }
    }

    let file = std::fs::File::open(path).ok()?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    Some(crate::duplicates::phash::dhash(&img))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_worker() -> ThumbnailWorker {
        // spawn() creates background threads that block waiting for requests.
        // We never send any requests, so they never touch GTK.
        // Drop sends Shutdown to all threads, so cleanup is automatic.
        let (worker, _result_rx, _hash_rx, _sharp_rx) = ThumbnailWorker::spawn(2, None);
        worker
    }

    #[test]
    fn bump_generation_increments_monotonically() {
        let worker = make_worker();
        let gen1 = worker.bump_generation();
        let gen2 = worker.bump_generation();
        let gen3 = worker.bump_generation();
        assert!(gen2 > gen1, "each bump must increase the generation");
        assert!(gen3 > gen2, "each bump must increase the generation");
    }

    #[test]
    fn current_generation_matches_last_bump() {
        let worker = make_worker();
        let bumped = worker.bump_generation();
        assert_eq!(worker.current_generation(), bumped);
    }

    #[test]
    fn pending_set_is_cleared_after_bump() {
        let worker = make_worker();
        {
            let arc = worker.pending_set();
            let mut pending = arc.lock().unwrap();
            pending.insert(PathBuf::from("/fake/a.jpg"));
            pending.insert(PathBuf::from("/fake/b.jpg"));
            assert_eq!(pending.len(), 2);
        }
        worker.bump_generation();
        let arc = worker.pending_set();
        let pending = arc.lock().unwrap();
        assert!(
            pending.is_empty(),
            "bump_generation must clear the in-flight dedup set"
        );
    }

    #[test]
    fn stale_request_generation_skipped_by_worker() {
        // Send a thumbnail request with an old generation. The worker must skip
        // it without producing a result. We can verify no result arrives within
        // a short timeout because the decode path is bypassed before any file I/O.
        let (worker, result_rx, _hash_rx, _sharp_rx) = ThumbnailWorker::spawn(2, None);
        let current_gen = worker.current_generation();

        // Bump so any request carrying current_gen is now stale.
        worker.bump_generation();

        let tx = worker.visible_sender();
        let _ = tx.send_blocking(WorkerRequest::Thumbnail {
            path: PathBuf::from("/nonexistent/stale.jpg"),
            gen: current_gen, // stale generation
        });

        // Give the worker a moment to process (it should skip immediately).
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            result_rx.try_recv().is_err(),
            "stale request must not produce a ThumbnailResult"
        );
    }

    #[test]
    fn jxl_thumbnail_worker_emits_result_for_real_sample() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("docs/wallpaper-37181-export-original-jxl-1.jxl");
        let (worker, result_rx, _hash_rx, _sharp_rx) = ThumbnailWorker::spawn(2, None);
        let gen = worker.current_generation();
        worker.visible_sender().send_blocking(WorkerRequest::Thumbnail {
            path: path.clone(),
            gen,
        }).unwrap();

        let started = std::time::Instant::now();
        loop {
            if let Ok(result) = result_rx.try_recv() {
                assert_eq!(result.path, path);
                assert!(result.width > 0 && result.height > 0);
                break;
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "timed out waiting for JXL thumbnail result"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn choose_thumbnail_webp_dimensions_scales_short_edge_to_target() {
        let (w, h) = choose_thumbnail_webp_dimensions(4000, 3000, THUMB_HEIGHT);
        assert_eq!((w, h), (213, 160));
    }

    #[test]
    fn choose_thumbnail_webp_dimensions_keeps_small_images() {
        let (w, h) = choose_thumbnail_webp_dimensions(120, 90, THUMB_HEIGHT);
        assert_eq!((w, h), (120, 90));
    }
}
