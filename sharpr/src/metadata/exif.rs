use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub(crate) fn rexiv2_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Display-ready snapshot of image metadata.
#[derive(Default, Debug, Clone)]
pub struct ImageMetadata {
    pub filename: String,
    pub file_size_bytes: u64,
    pub width: u32,
    pub height: u32,
    /// e.g. "JPEG", "PNG"
    pub format: String,
    /// ISO speed string, e.g. "400"
    pub iso: Option<String>,
    /// Shutter speed string, e.g. "1/500"
    pub shutter_speed: Option<String>,
    /// Aperture string, e.g. "f/2.8"
    pub aperture: Option<String>,
    /// Camera make + model string
    pub camera: Option<String>,
    /// Focal length string, e.g. "50 mm"
    pub focal_length: Option<String>,
    /// Lens model string, e.g. "EF50mm f/1.8 STM"
    pub lens: Option<String>,
    /// Colour space string, e.g. "sRGB"
    pub color_space: Option<String>,
}

impl ImageMetadata {
    /// Load metadata for `path`. Synchronous — call from a background thread.
    pub fn load(path: &Path) -> Self {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let file_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        let mut meta = Self {
            filename,
            file_size_bytes,
            format: extension_format(path),
            ..Default::default()
        };

        // rexiv2 0.10.0 is incompatible with the system gexiv2 0.16.0 — multiple
        // gexiv2 functions (get_tag_string, get_exif_tag_long, etc.) segfault.
        // Fall back to the image crate for dimensions until rexiv2 is updated.
        if let Some((w, h)) = read_dimensions_with_image(path) {
            meta.width = w;
            meta.height = h;
        }

        meta
    }

    /// Human-readable file size, e.g. "14.8 MB".
    pub fn file_size_display(&self) -> String {
        let b = self.file_size_bytes as f64;
        if b >= 1_000_000.0 {
            format!("{:.1} MB", b / 1_000_000.0)
        } else if b >= 1_000.0 {
            format!("{:.1} KB", b / 1_000.0)
        } else {
            format!("{} B", self.file_size_bytes)
        }
    }

    /// "6016 × 4016" string.
    pub fn dimensions_display(&self) -> Option<String> {
        if self.width > 0 && self.height > 0 {
            Some(format!("{} × {}", self.width, self.height))
        } else {
            None
        }
    }

    /// "24.1 MP" string.
    pub fn megapixels_display(&self) -> Option<String> {
        if self.width > 0 && self.height > 0 {
            let mp = (self.width as f64 * self.height as f64) / 1_000_000.0;
            Some(format!("{:.1} MP", mp))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extension_format(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_else(|| "Unknown".into())
}

fn read_dimensions_with_image(path: &Path) -> Option<(u32, u32)> {
    if crate::jxl::is_jxl_path(path) {
        return crate::jxl::image_dimensions(path).ok();
    }

    use image::ImageReader;
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path).ok()?;
    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    reader.into_dimensions().ok()
}
