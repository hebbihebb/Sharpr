use std::path::{Path, PathBuf};

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
        let _ = self
            .request_tx
            .send_blocking(WorkerRequest::Thumbnail(path));
    }

    /// Return a cloned sender so other components can submit requests directly.
    pub fn sender(&self) -> Sender<WorkerRequest> {
        self.request_tx.clone()
    }
}

impl Drop for ThumbnailWorker {
    fn drop(&mut self) {
        // Signal all threads to stop.
        for _ in 0..8 {
            let _ = self.request_tx.send_blocking(WorkerRequest::Shutdown);
        }
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust thumbnail generation (runs on worker threads, no GTK calls)
// ---------------------------------------------------------------------------

fn generate_thumbnail(path: &PathBuf) -> Option<ThumbnailResult> {
    use image::imageops::FilterType;
    use image::ImageFormat;
    use image::ImageReader;
    use std::fs::File;
    use std::io::BufReader;

    if let Some(result) = load_cached_thumbnail(path) {
        return Some(result);
    }

    let file = File::open(path).ok()?;
    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;

    // Apply EXIF orientation before resizing so portrait images render upright.
    let img = apply_exif_orientation(img, path);

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

    write_thumbnail_cache(path, &bytes, w, h, ImageFormat::Png);

    Some(ThumbnailResult {
        path: path.clone(),
        rgba_bytes: bytes,
        width: w,
        height: h,
    })
}

/// Read EXIF `Orientation` and rotate/flip the image so it displays upright.
/// Safe to call on worker threads — rexiv2 is initialised in main before any
/// threads are spawned.
fn apply_exif_orientation(img: image::DynamicImage, path: &Path) -> image::DynamicImage {
    use image::imageops;

    let orientation: u32 = rexiv2::Metadata::new_from_path(path)
        .ok()
        .and_then(|meta| meta.get_tag_string("Exif.Image.Orientation").ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1);

    match orientation {
        1 => img,
        2 => image::DynamicImage::ImageRgba8(imageops::flip_horizontal(&img.into_rgba8())),
        3 => image::DynamicImage::ImageRgba8(imageops::rotate180(&img.into_rgba8())),
        4 => image::DynamicImage::ImageRgba8(imageops::flip_vertical(&img.into_rgba8())),
        5 => image::DynamicImage::ImageRgba8(imageops::flip_horizontal(
            &imageops::rotate90(&img.into_rgba8()),
        )),
        6 => image::DynamicImage::ImageRgba8(imageops::rotate90(&img.into_rgba8())),
        7 => image::DynamicImage::ImageRgba8(imageops::flip_horizontal(
            &imageops::rotate270(&img.into_rgba8()),
        )),
        8 => image::DynamicImage::ImageRgba8(imageops::rotate270(&img.into_rgba8())),
        _ => img, // Unknown orientation — render as-is.
    }
}

fn load_cached_thumbnail(path: &Path) -> Option<ThumbnailResult> {
    use image::ImageReader;
    use std::fs::File;
    use std::io::BufReader;

    let cache_path = thumbnail_cache_path(path)?;
    let file = File::open(cache_path).ok()?;
    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    let rgba = img.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());

    Some(ThumbnailResult {
        path: path.to_path_buf(),
        rgba_bytes: rgba.into_raw(),
        width,
        height,
    })
}

fn write_thumbnail_cache(
    source_path: &Path,
    rgba_bytes: &[u8],
    width: u32,
    height: u32,
    format: image::ImageFormat,
) {
    let Some(cache_path) = thumbnail_cache_path(source_path) else {
        return;
    };
    let Some(parent) = cache_path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }

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
    let metadata = std::fs::metadata(source_path).ok()?;
    let modified = metadata.modified().ok()?;
    let modified = modified.duration_since(std::time::UNIX_EPOCH).ok()?;

    let cache_dir = thumbnail_cache_dir()?;
    let path_hash = stable_path_hash(source_path);
    let filename = format!(
        "{path_hash:016x}-{}-{}-{}.png",
        metadata.len(),
        modified.as_secs(),
        modified.subsec_nanos()
    );

    Some(cache_dir.join(filename))
}

fn thumbnail_cache_dir() -> Option<PathBuf> {
    if let Some(cache_home) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(cache_home).join("sharpr").join("thumbnails-r1"));
    }

    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("sharpr")
            .join("thumbnails-r1"), // r1 — includes EXIF rotation correction
    )
}

fn stable_path_hash(path: &Path) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in path.as_os_str().to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
