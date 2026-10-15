//! Wallpaper filters use displayed pixel dimensions, not resolution labels.
#[derive(Default)]
pub(crate) struct DimensionCache {
    entries: std::collections::HashMap<std::path::PathBuf, CachedDimensions>,
}
struct CachedDimensions {
    size: u64,
    modified: Option<std::time::SystemTime>,
    dimensions: Option<(u32, u32)>,
}
impl DimensionCache {
    pub fn get(
        &mut self,
        path: &std::path::Path,
        metadata: &std::fs::Metadata,
    ) -> Option<(u32, u32)> {
        let modified = metadata.modified().ok();
        if let Some(cached) = self.entries.get(path) {
            if cached.size == metadata.len() && cached.modified == modified {
                return cached.dimensions;
            }
        }
        let dimensions = crate::metadata::exif::display_dimensions(path);
        self.entries.insert(
            path.to_path_buf(),
            CachedDimensions {
                size: metadata.len(),
                modified,
                dimensions,
            },
        );
        dimensions
    }
}
pub fn widescreen_distance((w, h): (u32, u32)) -> f64 {
    ((w as f64 / h as f64) / (16.0 / 9.0) - 1.0).abs()
}

pub fn compare_aspects(
    a: Option<(u32, u32)>,
    b: Option<(u32, u32)>,
    closest: bool,
    descending: bool,
) -> std::cmp::Ordering {
    let key = |d: Option<(u32, u32)>| {
        d.filter(|(w, h)| *w > 0 && *h > 0).map(|(w, h)| {
            if closest {
                widescreen_distance((w, h))
            } else {
                w as f64 / h as f64
            }
        })
    };
    match (key(a), key(b)) {
        (Some(a), Some(b)) => {
            if descending {
                b.total_cmp(&a)
            } else {
                a.total_cmp(&b)
            }
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_aspects_sort_last_in_both_directions() {
        for desc in [false, true] {
            assert_eq!(
                compare_aspects(Some((16, 9)), None, false, desc),
                std::cmp::Ordering::Less
            );
        }
        assert_eq!(
            compare_aspects(Some((16, 9)), Some((4, 3)), true, false),
            std::cmp::Ordering::Less
        );
    }
}
