use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_channel::{Receiver, Sender};
use gtk4::gio;
use gtk4::prelude::*;

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
}

/// Target height for generated thumbnails (in pixels).
const THUMB_HEIGHT: u32 = 160;

impl ThumbnailWorker {
    /// Spawn background workers split into a visible pool (`thread_count - 1`
    /// threads) and a preload pool (1 thread).
    /// Returns the worker handle and the receivers for thumbnail and hash results.
    pub fn spawn(thread_count: usize) -> (Self, Receiver<ThumbnailResult>, Receiver<HashResult>) {
        let preload_workers = 4usize;
        let visible_workers = thread_count.saturating_sub(preload_workers).max(1);

        let (visible_tx, visible_rx) = async_channel::unbounded::<WorkerRequest>();
        let (preload_tx, preload_rx) = async_channel::unbounded::<WorkerRequest>();
        let (result_tx, result_rx) = async_channel::unbounded::<ThumbnailResult>();
        let (hash_result_tx, hash_result_rx) = async_channel::unbounded::<HashResult>();
        let generation = Arc::new(AtomicU64::new(0));
        let pending_paths = Arc::new(Mutex::new(HashSet::new()));

        // Visible workers — service in-viewport tile requests at normal priority.
        for _ in 0..visible_workers {
            let request_rx = visible_rx.clone();
            let request_tx = visible_tx.clone();
            let result_tx = result_tx.clone();
            let hash_result_tx = hash_result_tx.clone();
            let gen_arc = generation.clone();
            let pending_paths = pending_paths.clone();
            std::thread::spawn(move || {
                loop {
                    match request_rx.recv_blocking() {
                        Ok(WorkerRequest::Thumbnail { path, gen }) => {
                            // Skip stale requests immediately — no decode needed.
                            if gen != gen_arc.load(Ordering::Relaxed) {
                                if let Ok(mut pending) = pending_paths.lock() {
                                    pending.remove(&path);
                                }
                                continue;
                            }
                            if let Some(result) = generate_thumbnail(&path) {
                                let _ = result_tx.send_blocking(result);
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
                            if let Some(hash) = compute_hash(&path) {
                                let _ = hash_result_tx.send_blocking(HashResult { path, hash });
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
            let gen_arc = generation.clone();
            let pending_paths = pending_paths.clone();
            std::thread::spawn(move || loop {
                match request_rx.recv_blocking() {
                    Ok(WorkerRequest::Thumbnail { path, gen }) => {
                        if gen != gen_arc.load(Ordering::Relaxed) {
                            if let Ok(mut pending) = pending_paths.lock() {
                                pending.remove(&path);
                            }
                            continue;
                        }
                        if let Some(result) = generate_thumbnail(&path) {
                            let _ = result_tx.send_blocking(result);
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
                        if let Some(hash) = compute_hash(&path) {
                            let _ = hash_result_tx.send_blocking(HashResult { path, hash });
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
            },
            result_rx,
            hash_result_rx,
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
    // Try system-wide thumbnails first (GNOME/Freedesktop standard) as they
    // are likely already generated and high quality.
    if let Some(system_cached) = load_system_thumbnail(path) {
        return Some(system_cached);
    }

    // Fall back to Sharpr's own disk cache.
    if let Some(cached) = load_cached_thumbnail(path) {
        return Some(cached);
    }

    let is_jpeg = path
        .extension()
        .map(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "jpg" | "jpeg"))
        .unwrap_or(false);

    if is_jpeg {
        if let Some(img) = extract_exif_thumbnail(path) {
            let img = apply_exif_orientation(img, path);
            return build_thumbnail_and_cache(path, img);
        }
    }

    let img = if is_jpeg {
        // Try turbojpeg DCT-scale decode first (substantially faster for large JPEGs).
        decode_jpeg_scaled(path).or_else(|| decode_with_image_crate(path))?
    } else {
        decode_with_image_crate(path)?
    };
    let img = apply_exif_orientation(img, path);
    build_thumbnail_and_cache(path, img)
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

fn decode_with_image_crate(path: &Path) -> Option<image::DynamicImage> {
    let file = std::fs::File::open(path).ok()?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    reader.decode().ok()
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

    let resized = imageops::resize(&img.into_rgba8(), thumb_w, thumb_h, FilterType::Triangle);
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

fn load_system_thumbnail(path: &Path) -> Option<ThumbnailResult> {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let file = gio::File::for_path(&abs_path);
    
    let info = file.query_info(
        "thumbnail::path,thumbnail::is-valid",
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>
    ).ok()?;

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

fn apply_exif_orientation(img: image::DynamicImage, path: &Path) -> image::DynamicImage {
    use image::imageops;
    use image::DynamicImage;

    // Parse the EXIF orientation tag directly from the file bytes using a
    // minimal scan — avoids rexiv2/GExiv2 which is not thread-safe.
    let orientation: u32 = read_exif_orientation(path).unwrap_or(1);

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

fn extract_exif_thumbnail(path: &Path) -> Option<image::DynamicImage> {
    let (_, tiff) = read_exif_tiff(path)?;
    let tiff = tiff.as_slice();
    if tiff.len() < 8 {
        return None;
    }

    let little_endian = match &tiff[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };

    let read_u16 = |buf: &[u8], offset: usize| -> Option<u16> {
        let b = buf.get(offset..offset + 2)?;
        Some(if little_endian {
            u16::from_le_bytes(b.try_into().ok()?)
        } else {
            u16::from_be_bytes(b.try_into().ok()?)
        })
    };
    let read_u32 = |buf: &[u8], offset: usize| -> Option<u32> {
        let b = buf.get(offset..offset + 4)?;
        Some(if little_endian {
            u32::from_le_bytes(b.try_into().ok()?)
        } else {
            u32::from_be_bytes(b.try_into().ok()?)
        })
    };

    let ifd0_offset = read_u32(tiff, 4)? as usize;
    let entry_count = read_u16(tiff, ifd0_offset)? as usize;
    let next_ifd_ptr = ifd0_offset
        .checked_add(2)?
        .checked_add(entry_count.checked_mul(12)?)?;
    let ifd1_offset = read_u32(tiff, next_ifd_ptr)? as usize;
    if ifd1_offset == 0 || ifd1_offset >= tiff.len() {
        return None;
    }

    let ifd1_entries = read_u16(tiff, ifd1_offset)? as usize;
    let mut thumb_offset = None;
    let mut thumb_len = None;
    for i in 0..ifd1_entries {
        let entry_offset = ifd1_offset
            .checked_add(2)?
            .checked_add(i.checked_mul(12)?)?;
        let tag = read_u16(tiff, entry_offset)?;
        match tag {
            0x0201 => thumb_offset = Some(read_u32(tiff, entry_offset + 8)? as usize),
            0x0202 => thumb_len = Some(read_u32(tiff, entry_offset + 8)? as usize),
            _ => {}
        }
    }

    let thumb_offset = thumb_offset?;
    let thumb_len = thumb_len?;
    let thumb_end = thumb_offset.checked_add(thumb_len)?;
    let thumb_bytes = tiff.get(thumb_offset..thumb_end)?;
    let img = image::load_from_memory(thumb_bytes).ok()?;
    if img.height() < THUMB_HEIGHT {
        return None;
    }
    Some(img)
}

/// Read the EXIF Orientation tag (0x0112) from a JPEG file without using
/// rexiv2/GExiv2, which are not safe to call from multiple threads.
///
/// Returns `None` (caller treats as orientation 1 = normal) on any error.
fn read_exif_orientation(path: &Path) -> Option<u32> {
    let (little_endian, tiff) = read_exif_tiff(path)?;
    let tiff = tiff.as_slice();
    let read_u16 = |buf: &[u8], offset: usize| -> Option<u16> {
        let b = buf.get(offset..offset + 2)?;
        Some(if little_endian {
            u16::from_le_bytes(b.try_into().ok()?)
        } else {
            u16::from_be_bytes(b.try_into().ok()?)
        })
    };
    let read_u32 = |buf: &[u8], offset: usize| -> Option<u32> {
        let b = buf.get(offset..offset + 4)?;
        Some(if little_endian {
            u32::from_le_bytes(b.try_into().ok()?)
        } else {
            u32::from_be_bytes(b.try_into().ok()?)
        })
    };

    let ifd0_offset = read_u32(tiff, 4)? as usize;
    let entry_count = read_u16(tiff, ifd0_offset)? as usize;

    for i in 0..entry_count {
        let entry_offset = ifd0_offset.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        let tag = read_u16(tiff, entry_offset)?;
        if tag == 0x0112 {
            // Orientation tag: value is a SHORT stored at offset+8.
            let value = read_u16(tiff, entry_offset + 8)?;
            return Some(value as u32);
        }
    }
    None
}

fn read_exif_tiff(path: &Path) -> Option<(bool, Vec<u8>)> {
    use std::io::{BufReader, Read, Seek, SeekFrom};

    let file = std::fs::File::open(path).ok()?;
    let mut r = BufReader::new(file);

    let mut soi = [0u8; 2];
    r.read_exact(&mut soi).ok()?;
    if soi != [0xFF, 0xD8] {
        return None;
    }

    loop {
        let mut marker = [0u8; 2];
        r.read_exact(&mut marker).ok()?;
        if marker[0] != 0xFF {
            return None;
        }

        let marker_type = marker[1];
        if marker_type == 0xE1 {
            let mut len_buf = [0u8; 2];
            r.read_exact(&mut len_buf).ok()?;
            let segment_len = u16::from_be_bytes(len_buf) as usize;
            if segment_len < 2 {
                return None;
            }
            let data_len = segment_len - 2;
            let mut data = vec![0u8; data_len];
            r.read_exact(&mut data).ok()?;

            if data.len() < 6 || &data[..6] != b"Exif\0\0" {
                continue;
            }
            let tiff = data[6..].to_vec();
            if tiff.len() < 8 {
                return None;
            }

            let little_endian = match &tiff[..2] {
                b"II" => true,
                b"MM" => false,
                _ => return None,
            };
            return Some((little_endian, tiff));
        } else if marker_type == 0xDA || marker_type == 0xD9 {
            return None;
        } else {
            let mut len_buf = [0u8; 2];
            r.read_exact(&mut len_buf).ok()?;
            let segment_len = u16::from_be_bytes(len_buf) as usize;
            if segment_len < 2 {
                return None;
            }
            r.seek(SeekFrom::Current((segment_len - 2) as i64)).ok()?;
        }
    }
}

fn compute_hash(path: &Path) -> Option<u64> {
    // Optimization: if we have a thumbnail (from our cache or system cache),
    // use it for hashing instead of decoding the full source image.
    if let Some(thumb) = load_cached_thumbnail(path).or_else(|| load_system_thumbnail(path)) {
        if let Some(rgba) = image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba_bytes) {
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
