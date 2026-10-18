//! Shared bucket palette and drawing tokens. Theme surfaces live in data/style.css.
pub const BUCKETS: &[&str] = &[
    "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
];
/// Folder-icon colours of the library roots, in root order (D41): the first
/// root keeps the blue Pictures had, then orange, green, pink, teal, yellow.
pub const ROOTS: &[&str] = &[
    BUCKETS[1], BUCKETS[2], BUCKETS[0], BUCKETS[4], BUCKETS[5], BUCKETS[3],
];
pub type Ink = (f64, f64, f64, f64);
pub const SCRIM: Ink = (0.0, 0.0, 0.0, 0.6);
pub const WHITE: Ink = (1.0, 1.0, 1.0, 1.0);
pub const DIVIDER: Ink = (1.0, 1.0, 1.0, 0.9);
pub const CROP_GRID: Ink = (1.0, 1.0, 1.0, 0.35);
/// Dark outline under the 16:9 guide so it reads on light images (D36).
pub const GUIDE_EDGE_SHADOW: Ink = (0.0, 0.0, 0.0, 0.5);
pub const CROP_EDGE: Ink = (1.0, 1.0, 1.0, 0.95);
