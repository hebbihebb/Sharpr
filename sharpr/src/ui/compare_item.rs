use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

/// Which of the two images on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Choosing the best of several (decision D8): two images on screen at a
/// time; dismissing one brings in the next contender on its side. Whoever
/// hasn't been dismissed when Compare closes stays selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Knockout {
    left: PathBuf,
    right: Option<PathBuf>,
    waiting: std::collections::VecDeque<PathBuf>,
    dismissed: Vec<PathBuf>,
    total: usize,
}

impl Knockout {
    /// `None` for fewer than two images.
    pub fn new(paths: Vec<PathBuf>) -> Option<Self> {
        let total = paths.len();
        let mut waiting: std::collections::VecDeque<PathBuf> = paths.into();
        let left = waiting.pop_front()?;
        let right = Some(waiting.pop_front()?);
        Some(Self {
            left,
            right,
            waiting,
            dismissed: Vec::new(),
            total,
        })
    }

    pub fn left(&self) -> &Path {
        &self.left
    }

    /// `None` once a single image is left.
    pub fn right(&self) -> Option<&Path> {
        self.right.as_deref()
    }

    /// Dismisses one side; the next contender, if any, takes its place.
    /// Returns false when only one image was left to dismiss.
    pub fn dismiss(&mut self, side: Side) -> bool {
        let Some(right) = self.right.take() else {
            return false;
        };
        let next = self.waiting.pop_front();
        match side {
            Side::Left => {
                self.dismissed
                    .push(std::mem::replace(&mut self.left, right));
                self.right = next;
                // Keep the newcomer on the side that was dismissed.
                if let Some(newcomer) = self.right.take() {
                    self.right = Some(std::mem::replace(&mut self.left, newcomer));
                }
            }
            Side::Right => {
                self.dismissed.push(right);
                self.right = next;
            }
        }
        true
    }

    /// Brings back the last dismissed image on the side it left.
    pub fn undo(&mut self) -> bool {
        let Some(back) = self.dismissed.pop() else {
            return false;
        };
        if let Some(right) = self.right.take() {
            self.waiting.push_front(right);
        }
        self.right = Some(back);
        true
    }

    /// Images not dismissed, on screen or still waiting.
    pub fn survivors(&self) -> Vec<PathBuf> {
        std::iter::once(self.left.clone())
            .chain(self.right.clone())
            .chain(self.waiting.iter().cloned())
            .collect()
    }

    pub fn remaining(&self) -> usize {
        self.total - self.dismissed.len()
    }

    pub fn total(&self) -> usize {
        self.total
    }
}

/// The selected paths in filmstrip order, when there are at least two.
pub fn ordered_selection(
    filmstrip_paths: impl IntoIterator<Item = PathBuf>,
    selected: &HashSet<PathBuf>,
) -> Option<Vec<PathBuf>> {
    let ordered: Vec<PathBuf> = filmstrip_paths
        .into_iter()
        .filter(|path| selected.contains(path))
        .collect();
    (ordered.len() >= 2).then_some(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn five_images_narrow_down_to_one_through_knockouts() {
        let mut k = Knockout::new(paths(&["a", "b", "c", "d", "e"])).unwrap();
        assert_eq!(
            (k.left(), k.right()),
            (Path::new("a"), Some(Path::new("b")))
        );
        assert!(k.dismiss(Side::Left)); // c takes a's place
        assert_eq!(
            (k.left(), k.right()),
            (Path::new("c"), Some(Path::new("b")))
        );
        assert!(k.dismiss(Side::Right)); // d takes b's place
        assert_eq!(
            (k.left(), k.right()),
            (Path::new("c"), Some(Path::new("d")))
        );
        assert!(k.dismiss(Side::Left));
        assert_eq!(
            (k.left(), k.right()),
            (Path::new("e"), Some(Path::new("d")))
        );
        assert_eq!(k.remaining(), 2);
        assert!(k.dismiss(Side::Right));
        assert_eq!((k.left(), k.right()), (Path::new("e"), None));
        assert!(!k.dismiss(Side::Left), "the last image stays");
        assert_eq!(k.survivors(), paths(&["e"]));
        assert!(k.undo());
        assert_eq!(
            (k.left(), k.right()),
            (Path::new("e"), Some(Path::new("d")))
        );
    }

    #[test]
    fn closing_early_keeps_everyone_not_dismissed() {
        let mut k = Knockout::new(paths(&["a", "b", "c"])).unwrap();
        k.dismiss(Side::Right);
        assert_eq!(k.survivors(), paths(&["a", "c"]));
        assert!(Knockout::new(paths(&["a"])).is_none());
        let selected = HashSet::from([PathBuf::from("c"), PathBuf::from("a")]);
        assert_eq!(
            ordered_selection(paths(&["a", "b", "c"]), &selected),
            Some(paths(&["a", "c"]))
        );
    }
}
