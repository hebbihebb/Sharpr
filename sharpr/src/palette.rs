//! Shared bucket palette and drawing tokens. Theme surfaces live in data/style.css.
pub const BUCKETS: &[&str] = &[
    "#57e389", "#62a0ea", "#ff7800", "#f5c211", "#dc8add", "#5bc8af", "#e01b24", "#9141ac",
];
pub type Ink = (f64, f64, f64, f64);
pub const SCRIM: Ink = (0.0, 0.0, 0.0, 0.6);
pub const WHITE: Ink = (1.0, 1.0, 1.0, 1.0);
pub const DIVIDER: Ink = (1.0, 1.0, 1.0, 0.9);
pub const CROP_GRID: Ink = (1.0, 1.0, 1.0, 0.35);
/// Dark outline under the 16:9 guide so it reads on light images (D36).
pub const GUIDE_EDGE_SHADOW: Ink = (0.0, 0.0, 0.0, 0.5);
pub const CROP_EDGE: Ink = (1.0, 1.0, 1.0, 0.95);
