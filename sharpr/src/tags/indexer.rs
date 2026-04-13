use std::path::Path;

use crate::metadata::exif::ImageMetadata;

/// Returns a small set of automatic tags derived from the file's format and
/// pixel dimensions. These are inserted without overwriting user-created tags.
pub fn auto_tags(path: &Path, meta: &ImageMetadata) -> Vec<String> {
    let mut tags = Vec::new();

    // Format tag from file extension (e.g. "jpg", "png", "webp").
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_lowercase();
        if !ext.is_empty() {
            tags.push(ext);
        }
    }

    // Resolution bucket based on the longer edge.
    let long = meta.width.max(meta.height);
    if long >= 3840 {
        tags.push("4k".to_string());
    } else if long >= 1920 {
        tags.push("1080p".to_string());
    } else if long >= 1280 {
        tags.push("720p".to_string());
    }

    tags
}
