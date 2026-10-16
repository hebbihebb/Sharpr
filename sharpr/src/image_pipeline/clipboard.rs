//! Serial, coalescing clipboard decoding. No GTK objects cross this boundary.
use async_channel::{Receiver, Sender};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
struct Request {
    path: PathBuf,
    generation: u64,
}
pub struct ResultImage {
    pub path: PathBuf,
    pub generation: u64,
    pub image: Result<Image, String>,
}
pub struct Decoder {
    tx: Sender<Request>,
    pending: Receiver<Request>,
    generation: Arc<AtomicU64>,
}

fn decode(path: &Path) -> Result<Image, String> {
    let image = crate::export::decode_oriented(path)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    Ok(Image {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

impl Decoder {
    pub fn start() -> (Self, Receiver<ResultImage>) {
        Self::start_with(decode)
    }

    fn start_with(
        decode: impl Fn(&Path) -> Result<Image, String> + Send + 'static,
    ) -> (Self, Receiver<ResultImage>) {
        let (tx, pending) = async_channel::bounded::<Request>(1);
        let requests = pending.clone();
        let (results, rx) = async_channel::bounded(1);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv_blocking() {
                if current.load(Ordering::Relaxed) != request.generation {
                    continue;
                }
                let image = decode(&request.path);
                if current.load(Ordering::Relaxed) != request.generation {
                    continue;
                }
                if results
                    .send_blocking(ResultImage {
                        path: request.path,
                        generation: request.generation,
                        image,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        (
            Self {
                tx,
                pending,
                generation,
            },
            rx,
        )
    }

    /// One running decode and at most one queued request. Repeated clicks
    /// replace the queued request and retire older results without blocking GTK.
    pub fn request(&self, path: PathBuf) {
        let generation = self
            .generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let mut request = Request { path, generation };
        loop {
            match self.tx.try_send(request) {
                Ok(()) | Err(async_channel::TrySendError::Closed(_)) => return,
                Err(async_channel::TrySendError::Full(returned)) => {
                    let _ = self.pending.try_recv();
                    request = returned;
                }
            }
        }
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == generation
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.tx.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_requests_do_not_wait_for_decoding_and_only_the_latest_result_survives() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (decoder, results) = Decoder::start_with(move |path| {
            started_tx
                .send((path.to_path_buf(), std::thread::current().id()))
                .unwrap();
            if path == Path::new("first") {
                release_rx.recv().unwrap();
            }
            Ok(Image {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255],
            })
        });
        decoder.request("first".into());
        let timeout = std::time::Duration::from_secs(5);
        let (path, thread) = started_rx.recv_timeout(timeout).unwrap();
        assert_eq!(path, Path::new("first"));
        assert_ne!(thread, std::thread::current().id());
        // These return while the decoder remains held on the first request.
        decoder.request("superseded".into());
        decoder.request("latest".into());
        assert!(started_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        assert_eq!(
            started_rx.recv_timeout(timeout).unwrap().0,
            Path::new("latest")
        );
        let result = results.recv_blocking().unwrap();
        assert_eq!(result.path, Path::new("latest"));
        assert!(decoder.is_current(result.generation));
        assert!(results.try_recv().is_err());
    }

    #[test]
    fn clipboard_decoding_is_oriented_and_preserves_source_bytes() {
        use image::ImageEncoder;
        let root = std::env::temp_dir().join(format!("sharpr-clipboard-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("oriented.png");
        let pixels = image::RgbImage::from_fn(32, 18, |x, y| image::Rgb([x as u8, y as u8, 100]));
        let mut encoder =
            image::codecs::png::PngEncoder::new(std::fs::File::create(&path).unwrap());
        encoder
            .set_exif_metadata(vec![
                b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0,
                0, 0,
            ])
            .unwrap();
        encoder
            .write_image(pixels.as_raw(), 32, 18, image::ExtendedColorType::Rgb8)
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let image = decode(&path).unwrap();
        assert_eq!((image.width, image.height), (18, 32));
        assert_eq!(&image.rgba[..4], &[0, 17, 100, 255]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let broken = root.join("broken.png");
        std::fs::write(&broken, b"invalid").unwrap();
        assert!(decode(&broken).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
