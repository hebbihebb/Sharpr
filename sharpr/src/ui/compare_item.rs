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
/// time; dismissing one brings in the next contender on its side, and
/// either side can show any image not yet dismissed. Whoever hasn't been
/// dismissed when Compare closes stays selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Knockout {
    /// Every image, in filmstrip order; positions are their numbers.
    order: Vec<PathBuf>,
    left: usize,
    right: Option<usize>,
    /// Dismissed images, latest last, with the side each left from.
    dismissed: Vec<(usize, Side)>,
}

impl Knockout {
    /// `None` for fewer than two images.
    pub fn new(paths: Vec<PathBuf>) -> Option<Self> {
        (paths.len() >= 2).then(|| Self {
            order: paths,
            left: 0,
            right: Some(1),
            dismissed: Vec::new(),
        })
    }

    pub fn left(&self) -> &Path {
        &self.order[self.left]
    }

    /// `None` once a single image is left.
    pub fn right(&self) -> Option<&Path> {
        self.right.map(|i| self.order[i].as_path())
    }

    /// The number (from 0) of the image on `side`.
    pub fn shown(&self, side: Side) -> Option<usize> {
        match side {
            Side::Left => Some(self.left),
            Side::Right => self.right,
        }
    }

    fn is_dismissed(&self, i: usize) -> bool {
        self.dismissed.iter().any(|(d, _)| *d == i)
    }

    /// The images not dismissed, with their numbers, in order.
    pub fn candidates(&self) -> Vec<(usize, &Path)> {
        self.order
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.is_dismissed(*i))
            .map(|(i, p)| (i, p.as_path()))
            .collect()
    }

    /// The first image neither on screen nor dismissed.
    fn next_waiting(&self) -> Option<usize> {
        (0..self.order.len())
            .find(|i| *i != self.left && Some(*i) != self.right && !self.is_dismissed(*i))
    }

    /// Dismisses one side; the next contender, if any, takes its place.
    /// Returns false when only one image was left to dismiss.
    pub fn dismiss(&mut self, side: Side) -> bool {
        let Some(right) = self.right else {
            return false;
        };
        match side {
            Side::Left => {
                self.dismissed.push((self.left, Side::Left));
                match self.next_waiting() {
                    Some(next) => self.left = next,
                    None => {
                        self.left = right;
                        self.right = None;
                    }
                }
            }
            Side::Right => {
                self.dismissed.push((right, Side::Right));
                self.right = self.next_waiting();
            }
        }
        true
    }

    /// Shows image `number` on `side`. The image on the other side swaps
    /// over; a dismissed image can't be chosen.
    pub fn choose(&mut self, side: Side, number: usize) -> bool {
        if number >= self.order.len() || self.is_dismissed(number) {
            return false;
        }
        let Some(right) = self.right else {
            return false;
        };
        match side {
            Side::Left if number == right => (self.left, self.right) = (right, Some(self.left)),
            Side::Left => self.left = number,
            Side::Right if number == self.left => {
                (self.left, self.right) = (right, Some(self.left))
            }
            Side::Right => self.right = Some(number),
        }
        true
    }

    /// Brings back the last dismissed image on the side it left.
    pub fn undo(&mut self) -> bool {
        let Some((back, side)) = self.dismissed.pop() else {
            return false;
        };
        match (side, self.right) {
            (Side::Left, Some(_)) => self.left = back,
            (Side::Left, None) => {
                self.right = Some(self.left);
                self.left = back;
            }
            (Side::Right, _) => self.right = Some(back),
        }
        true
    }

    /// Images not dismissed, in order.
    pub fn survivors(&self) -> Vec<PathBuf> {
        self.candidates()
            .into_iter()
            .map(|(_, p)| p.to_path_buf())
            .collect()
    }

    pub fn remaining(&self) -> usize {
        self.order.len() - self.dismissed.len()
    }

    pub fn total(&self) -> usize {
        self.order.len()
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
    fn either_side_can_show_any_image_not_dismissed() {
        let mut k = Knockout::new(paths(&["a", "b", "c", "d"])).unwrap();
        assert!(k.choose(Side::Right, 3));
        assert_eq!(
            (k.left(), k.right()),
            (Path::new("a"), Some(Path::new("d")))
        );
        // Choosing the other side's image swaps them.
        assert!(k.choose(Side::Left, 3));
        assert_eq!(
            (k.left(), k.right()),
            (Path::new("d"), Some(Path::new("a")))
        );
        assert!(k.dismiss(Side::Right)); // a goes; b is the next waiting
        assert_eq!(k.right(), Some(Path::new("b")));
        assert!(
            !k.choose(Side::Left, 0),
            "a dismissed image can't come back this way"
        );
        assert_eq!(
            k.candidates().iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(k.undo());
        assert_eq!(k.right(), Some(Path::new("a")));
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
