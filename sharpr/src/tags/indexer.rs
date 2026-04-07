use std::collections::HashSet;
use std::path::Path;

use crate::metadata::exif::ImageMetadata;

pub fn index_entry(path: &Path, meta: &ImageMetadata) -> Vec<String> {
    let mut tags: HashSet<String> = Default::default();

    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        for token in stem.split(|c: char| !c.is_alphanumeric()) {
            let token = token.to_lowercase();
            if token.len() >= 3 && !token.chars().all(|c| c.is_ascii_digit()) {
                tags.insert(token);
            }
        }
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        tags.insert(ext.to_lowercase());
    }

    if let Some(camera) = meta.camera.as_deref() {
        for word in camera.split_whitespace() {
            let word = word.to_lowercase();
            if word.len() >= 2 {
                tags.insert(word);
            }
        }
    }

    if let Some(lens) = meta.lens.as_deref() {
        for word in lens.split_whitespace() {
            let word = word.to_lowercase();
            if word.len() >= 2 {
                tags.insert(word);
            }
        }
    }

    if let Some(focal_length) = meta.focal_length.as_deref().and_then(parse_focal_length) {
        tags.insert(focal_bucket(focal_length).to_string());
    }

    if let Some(iso) = meta.iso.as_deref().and_then(|iso| iso.parse::<u32>().ok()) {
        tags.insert(if iso <= 800 { "low-iso" } else { "high-iso" }.to_string());
    }

    if let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) {
        use std::time::UNIX_EPOCH;
        if let Ok(secs) = mtime.duration_since(UNIX_EPOCH) {
            let year = 1970 + secs.as_secs() / 31_557_600;
            tags.insert(year.to_string());
        }
    }

    let mut tags: Vec<_> = tags.into_iter().collect();
    tags.sort();
    tags
}

fn parse_focal_length(s: &str) -> Option<f32> {
    s.split_whitespace().next()?.parse().ok()
}

fn focal_bucket(mm: f32) -> &'static str {
    if mm < 24.0 {
        "ultra-wide"
    } else if mm < 50.0 {
        "wide"
    } else if mm < 85.0 {
        "normal"
    } else if mm < 200.0 {
        "portrait"
    } else {
        "tele"
    }
}
