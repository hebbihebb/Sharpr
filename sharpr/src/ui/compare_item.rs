use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareAssetInfo {
    pub title: String,
    pub path: PathBuf,
    pub format: String,
    pub dimensions: Option<(u32, u32)>,
    pub file_size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareItem {
    pub left: CompareAssetInfo,
    pub right: CompareAssetInfo,
}

impl CompareItem {
    pub fn from_paths(left: PathBuf, right: PathBuf) -> Self {
        Self {
            left: build_compare_asset_info("Left", left),
            right: build_compare_asset_info("Right", right),
        }
    }
}

pub fn ordered_selected_paths(
    filmstrip_paths: impl IntoIterator<Item = PathBuf>,
    selected: &HashSet<PathBuf>,
) -> Option<(PathBuf, PathBuf)> {
    if selected.len() != 2 {
        return None;
    }
    let mut ordered = filmstrip_paths
        .into_iter()
        .filter(|path| selected.contains(path));
    let left = ordered.next()?;
    let right = ordered.next()?;
    ordered.next().is_none().then_some((left, right))
}

fn build_compare_asset_info(title: &str, path: PathBuf) -> CompareAssetInfo {
    CompareAssetInfo {
        title: title.to_string(),
        format: compare_asset_format(&path),
        dimensions: image::image_dimensions(&path).ok(),
        file_size: std::fs::metadata(&path).ok().map(|meta| meta.len()),
        path,
    }
}

pub fn compare_asset_format(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "JPEG".to_string(),
        Some("png") => "PNG".to_string(),
        Some("webp") => "WebP".to_string(),
        Some("jxl") => "JPEG XL".to_string(),
        Some("avif") => "AVIF".to_string(),
        Some("tif") | Some("tiff") => "TIFF".to_string(),
        Some("bmp") => "BMP".to_string(),
        Some("gif") => "GIF".to_string(),
        Some(other) => other.to_ascii_uppercase(),
        None => "Unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_requires_exactly_two_selected_paths() {
        let paths = vec![PathBuf::from("a.jpg"), PathBuf::from("b.jpg")];
        assert!(ordered_selected_paths(paths.clone(), &HashSet::new()).is_none());
        assert!(ordered_selected_paths(paths, &HashSet::from([PathBuf::from("a.jpg")])).is_none());
    }

    #[test]
    fn compare_uses_filmstrip_order() {
        let paths = vec![
            PathBuf::from("c.jpg"),
            PathBuf::from("a.jpg"),
            PathBuf::from("b.jpg"),
        ];
        let selected = HashSet::from([PathBuf::from("b.jpg"), PathBuf::from("c.jpg")]);
        assert_eq!(
            ordered_selected_paths(paths, &selected),
            Some((PathBuf::from("c.jpg"), PathBuf::from("b.jpg")))
        );
    }
}
