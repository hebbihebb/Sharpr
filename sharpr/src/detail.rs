//! Real detail (D48): how much detail an image actually holds, as a
//! resolution, so fake 4K (4K pixels upscaled from less) and soft images can
//! be found, each with a reason in plain words.
//!
//! The method is a round trip on luminance. For each size on a ladder of
//! short sides (540, 720, 1080, …) the image is scaled down to that size and
//! back up, and what the round trip loses is measured in textured tiles only.
//! An image upscaled from 1080p loses almost nothing in a round trip through
//! 1080p; a native image loses a share that grows with the scale. Each tile's
//! loss is divided by that tile's own texture (what a round trip through the
//! smallest size loses), so busy and quiet tiles weigh alike, and the median
//! over the tiles decides. Flat areas (sky, gradients, minimal art) hold no
//! detail at any size, so they are not measured; an image with too little
//! texture is "not measurable", never "low".
//!
//! Known limit: an AI upscaler that invents fine detail is not caught; only
//! interpolated and soft upscales are.
//!
//! Same image, same answer: there is no randomness, the windows and tiles
//! are taken in a fixed order and the decision is a median.

use std::path::Path;

use image::imageops::{self, FilterType};
use image::{ImageBuffer, Luma};

/// Changes whenever the method or its constants change, so cached results
/// from an older method are measured again.
pub const VERSION: u32 = 1;

/// Short sides tried, smallest first. The first one that fits under the
/// image is the reference for each tile's texture.
const LADDER: [u32; 10] = [540, 720, 1080, 1440, 2160, 2880, 4320, 5760, 8640, 11520];
/// A ladder size counts only if it is clearly below the image's own short
/// side; otherwise it is the image's own size.
const BELOW_OWN: f64 = 0.9;
/// The smallest short side measured at all.
const MIN_SHORT_SIDE: u32 = 600;
/// Tile side, in image pixels.
const TILE: usize = 64;
/// Tiles per window side; the window adds a margin for the filters.
const WINDOW_TILES: usize = 4;
/// At most this many windows are measured, the most textured first, so a
/// 270-megapixel image costs about as much as a 4K one.
const MAX_WINDOWS: usize = 64;
/// A tile is textured when a round trip through the smallest size loses at
/// least this much (mean square, luminance 0–1): about 1.5 levels of 255.
const TEXTURE_FLOOR: f64 = 3.5e-5;
/// Fewer textured tiles than this: not measurable.
const MIN_TILES: usize = 16;
/// A round trip through a size "loses almost nothing" when its median loss,
/// relative to the tile's texture, is at most this share of what a native
/// image would lose there (detail spread evenly over the octaves).
const NEAR_LOSSLESS: f64 = 0.25;
/// A ladder size is tested by a round trip slightly above it, because a
/// round trip also softens what lies just under its own limit.
const PROBE: f64 = 1.4;
/// The round trip's filter, both ways.
const FILTER: FilterType = FilterType::Lanczos3;

/// Why an image could not be measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmeasurable {
    /// Too few pixels to have a ladder below them.
    TooSmall,
    /// Too few textured areas to judge (flat art, sky, gradients).
    LittleTexture,
}

/// What the measurement found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estimate {
    /// The detail reaches the image's own size.
    Full,
    /// The detail looks no finer than this short side, a ladder size below
    /// the image's own: the image looks upscaled.
    Below {
        short_side: u32,
    },
    NotMeasurable(Unmeasurable),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    /// The image's own (displayed) size.
    pub width: u32,
    pub height: u32,
    pub estimate: Estimate,
    /// How many tiles were textured enough to measure.
    pub textured_tiles: usize,
    /// For each ladder size above the smallest: (short side, median loss
    /// relative to texture, the most a near-lossless size may lose).
    pub losses: Vec<(u32, f64, f64)>,
    /// Top-left corner of the most textured window, for showing a crop.
    pub focus: Option<(u32, u32)>,
}

impl Detail {
    /// The detail as a size with the image's aspect: its own size when the
    /// detail is full, `None` when it couldn't be measured.
    pub fn detail_size(&self) -> Option<(u32, u32)> {
        match self.estimate {
            Estimate::Full => Some((self.width, self.height)),
            Estimate::Below { short_side } => {
                Some(scaled_size(self.width, self.height, short_side))
            }
            Estimate::NotMeasurable(_) => None,
        }
    }

    /// "Looks upscaled" (D48): measured detail clearly below the pixel size,
    /// at least one ladder size under it, at any resolution.
    pub fn looks_upscaled(&self) -> bool {
        matches!(self.estimate, Estimate::Below { .. })
    }
}

/// What is kept of a measurement in the index's cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Full,
    Below,
    LittleTexture,
    TooSmall,
    /// The image couldn't be decoded; not tried again until it changes.
    Failed,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Below => "below",
            Self::LittleTexture => "little-texture",
            Self::TooSmall => "too-small",
            Self::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [
            Self::Full,
            Self::Below,
            Self::LittleTexture,
            Self::TooSmall,
            Self::Failed,
        ]
        .into_iter()
        .find(|s| s.key() == key)
    }
}

/// A cached measurement: the image's size and, when measured, its detail
/// as a size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored {
    pub status: Status,
    pub size: (u32, u32),
    pub detail: Option<(u32, u32)>,
}

impl Stored {
    pub fn from_detail(detail: &Detail) -> Self {
        Self {
            status: match detail.estimate {
                Estimate::Full => Status::Full,
                Estimate::Below { .. } => Status::Below,
                Estimate::NotMeasurable(Unmeasurable::LittleTexture) => Status::LittleTexture,
                Estimate::NotMeasurable(Unmeasurable::TooSmall) => Status::TooSmall,
            },
            size: (detail.width, detail.height),
            detail: detail.detail_size(),
        }
    }

    pub fn failed() -> Self {
        Self {
            status: Status::Failed,
            size: (0, 0),
            detail: None,
        }
    }

    /// "Looks upscaled" (D48): measured detail clearly below the pixel size.
    pub fn looks_upscaled(&self) -> bool {
        self.status == Status::Below
    }

    /// The detail's short side, for sorting; `None` when not measurable.
    pub fn detail_short_side(&self) -> Option<u32> {
        self.detail.map(|(w, h)| w.min(h))
    }

    /// The Info line, in plain words.
    pub fn words(&self) -> String {
        match (self.status, self.detail) {
            (Status::Full, _) => format!("Real detail: full {}×{}", self.size.0, self.size.1),
            (Status::Below, Some((w, h))) => format!("Real detail ≈ {w}×{h} · looks upscaled"),
            (Status::TooSmall, _) => "Real detail: can't tell (too small to measure)".into(),
            (Status::Failed, _) => "Real detail: couldn't read the image".into(),
            _ => "Real detail: can't tell (too little texture)".into(),
        }
    }
}

/// `width`×`height` scaled so its short side is `short_side`.
pub fn scaled_size(width: u32, height: u32, short_side: u32) -> (u32, u32) {
    let own = width.min(height).max(1);
    let scale =
        |v: u32| ((f64::from(v) * f64::from(short_side) / f64::from(own)).round() as u32).max(1);
    (scale(width), scale(height))
}

/// Decodes `path` at full size and measures it. Runs on a worker; never on
/// the GTK thread.
pub fn measure_path(path: &Path) -> Result<Detail, String> {
    let image = crate::export::decode_oriented(path).map_err(|e| e.to_string())?;
    Ok(measure_image(&image))
}

pub fn measure_image(image: &image::DynamicImage) -> Detail {
    let luma = image.to_luma32f();
    let (width, height) = luma.dimensions();
    measure_luma(&luma, width, height)
}

type Plane = ImageBuffer<Luma<f32>, Vec<f32>>;

pub fn measure_luma(luma: &Plane, width: u32, height: u32) -> Detail {
    measure_luma_probed(luma, width, height, PROBE)
}

/// `measure_luma` with another probe factor, for the gate's tuning.
fn measure_luma_probed(luma: &Plane, width: u32, height: u32, probe_factor: f64) -> Detail {
    let unmeasurable = |why, tiles| Detail {
        width,
        height,
        estimate: Estimate::NotMeasurable(why),
        textured_tiles: tiles,
        losses: Vec::new(),
        focus: None,
    };
    let short = width.min(height);
    // The smallest ladder size is the texture reference; each larger one is
    // probed a little above itself, and both must sit below the image.
    let below = |v: f64| v < BELOW_OWN * f64::from(short);
    let mut ladder: Vec<u32> = LADDER
        .iter()
        .copied()
        .filter(|&l| below(f64::from(l)))
        .collect();
    let probed = ladder
        .iter()
        .skip(1)
        .take_while(|&&l| below(f64::from(l) * probe_factor))
        .count();
    ladder.truncate(1 + probed);
    if short < MIN_SHORT_SIDE || ladder.len() < 2 {
        return unmeasurable(Unmeasurable::TooSmall, 0);
    }
    // The downscale to the smallest size reads about 2/scale pixels either
    // side; the margin keeps window edges out of the measured tiles.
    let smallest_scale = f64::from(ladder[0]) / f64::from(short);
    let margin = ((3.0 / smallest_scale).ceil() as usize + 8).max(32);
    let inner = TILE * WINDOW_TILES;
    let windows = choose_windows(luma, inner, margin);
    let Some(&(_, fx, fy)) = windows.first() else {
        return unmeasurable(Unmeasurable::LittleTexture, 0);
    };
    let focus = Some(((fx + margin) as u32, (fy + margin) as u32));
    let mut ordered: Vec<(usize, usize)> = windows.iter().map(|&(_, x, y)| (x, y)).collect();
    ordered.sort_unstable_by_key(|&(x, y)| (y, x));

    // Per window: per tile, the loss of each ladder size's round trip.
    // Sequential on purpose: the background pass runs this on its own
    // nice-19 thread, and the shared rayon pool would run it at normal
    // priority beside the thumbnail workers.
    let per_window: Vec<Vec<Vec<f64>>> = ordered
        .iter()
        .map(|&(x, y)| {
            let side = (inner + 2 * margin) as u32;
            let window = imageops::crop_imm(luma, x as u32, y as u32, side, side).to_image();
            round_trip_losses(&window, &ladder, short, margin, inner, probe_factor)
        })
        .collect();

    let textured: Vec<&Vec<f64>> = per_window
        .iter()
        .flatten()
        .filter(|losses| losses[0] >= TEXTURE_FLOOR)
        .collect();
    if textured.len() < MIN_TILES {
        return unmeasurable(Unmeasurable::LittleTexture, textured.len());
    }

    let span = (f64::from(short) / f64::from(ladder[0])).ln();
    let mut losses = Vec::new();
    let mut estimate = Estimate::Full;
    for (i, &level) in ladder.iter().enumerate().skip(1) {
        let mut ratios: Vec<f64> = textured.iter().map(|t| t[i] / t[0]).collect();
        let median = median(&mut ratios);
        let native_share = (f64::from(short) / probe(level, i, probe_factor)).ln() / span;
        let allowed = NEAR_LOSSLESS * native_share;
        losses.push((level, median, allowed));
        if median <= allowed && estimate == Estimate::Full {
            estimate = Estimate::Below { short_side: level };
        }
    }
    Detail {
        width,
        height,
        estimate,
        textured_tiles: textured.len(),
        losses,
        focus,
    }
}

/// The windows to measure: a grid of `inner`-sized squares with `margin`
/// around them, the `MAX_WINDOWS` with the most edge energy, as
/// (energy, left, top) with the margin's corner. Ties keep grid order.
fn choose_windows(luma: &Plane, inner: usize, margin: usize) -> Vec<(f64, usize, usize)> {
    let (w, h) = (luma.width() as usize, luma.height() as usize);
    let side = inner + 2 * margin;
    if w < side || h < side {
        return Vec::new();
    }
    let data = luma.as_raw();
    let mut corners = Vec::new();
    let mut y = 0;
    while y + side <= h {
        let mut x = 0;
        while x + side <= w {
            corners.push((x, y));
            x += inner;
        }
        y += inner;
    }
    let mut scored: Vec<(f64, usize, usize)> = corners
        .iter()
        .map(|&(x, y)| {
            let mut energy = 0.0f64;
            for row in (y + margin)..(y + margin + inner) {
                let line = &data[row * w..row * w + w];
                let below = &data[(row + 1).min(h - 1) * w..(row + 1).min(h - 1) * w + w];
                for col in (x + margin)..(x + margin + inner) {
                    energy += f64::from((line[col + 1] - line[col]).abs())
                        + f64::from((below[col] - line[col]).abs());
                }
            }
            (energy, x, y)
        })
        .collect();
    // Most energy first; equal energy keeps grid order (stable sort).
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.truncate(MAX_WINDOWS);
    scored
}

/// For each tile inside `window`'s margin, the mean square loss of a round
/// trip through each ladder size.
fn round_trip_losses(
    window: &Plane,
    ladder: &[u32],
    short: u32,
    margin: usize,
    inner: usize,
    probe_factor: f64,
) -> Vec<Vec<f64>> {
    let side = window.width();
    let tiles = inner / TILE;
    let mut out = vec![vec![0.0; ladder.len()]; tiles * tiles];
    for (i, &level) in ladder.iter().enumerate() {
        let small = ((f64::from(side) * probe(level, i, probe_factor) / f64::from(short)).round()
            as u32)
            .max(4);
        let down = imageops::resize(window, small, small, FILTER);
        let back = imageops::resize(&down, side, side, FILTER);
        let (a, b) = (window.as_raw(), back.as_raw());
        for ty in 0..tiles {
            for tx in 0..tiles {
                let mut sum = 0.0f64;
                for row in 0..TILE {
                    let start = (margin + ty * TILE + row) * side as usize + margin + tx * TILE;
                    for k in start..start + TILE {
                        let d = f64::from(a[k] - b[k]);
                        sum += d * d;
                    }
                }
                out[ty * tiles + tx][i] = sum / (TILE * TILE) as f64;
            }
        }
    }
    out
}

/// The size a ladder size is tested at: the smallest (the texture
/// reference) as it is, the others a little above.
fn probe(level: u32, index: usize, factor: f64) -> f64 {
    if index == 0 {
        f64::from(level)
    } else {
        f64::from(level) * factor
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// The detail in plain words, for the gate's sheets.
#[cfg(test)]
pub fn describe(detail: &Detail) -> String {
    Stored::from_detail(detail).words()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    /// A deterministic, richly textured image: sums of sines at many
    /// frequencies up to the pixel limit, plus hashed noise.
    fn textured(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let mut v = 0.0f32;
            for k in 0..12 {
                let f = 0.002 * 1.6f32.powi(k);
                v += (fx * f + fy * f * 0.7 + k as f32).sin() * (1.0 / (1.0 + k as f32 * 0.15));
            }
            let hash = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 97;
            let value = (128.0 + v * 9.0 + hash as f32 - 48.0).clamp(0.0, 255.0) as u8;
            image::Rgb([value, value.wrapping_add(10), value])
        })
    }

    fn fake(native: &RgbImage, short_side: u32) -> RgbImage {
        let (w, h) = scaled_size(native.width(), native.height(), short_side);
        let small = imageops::resize(native, w, h, FilterType::Lanczos3);
        imageops::resize(
            &small,
            native.width(),
            native.height(),
            FilterType::Triangle,
        )
    }

    #[test]
    fn native_texture_is_full_detail() {
        let image = DynamicImage::ImageRgb8(textured(2560, 1440));
        let detail = measure_image(&image);
        assert_eq!(detail.estimate, Estimate::Full, "{detail:?}");
        assert!(!detail.looks_upscaled());
        assert_eq!(detail.detail_size(), Some((2560, 1440)));
    }

    #[test]
    fn an_upscale_from_720p_looks_upscaled() {
        let native = textured(2560, 1440);
        let detail = measure_image(&DynamicImage::ImageRgb8(fake(&native, 720)));
        assert!(detail.looks_upscaled(), "{detail:?}");
        assert_eq!(detail.detail_size(), Some((1280, 720)));
        assert_eq!(describe(&detail), "Real detail ≈ 1280×720 · looks upscaled");
    }

    #[test]
    fn a_1080p_image_is_too_small_to_tell() {
        let detail = measure_image(&DynamicImage::ImageRgb8(textured(1920, 1080)));
        assert_eq!(
            detail.estimate,
            Estimate::NotMeasurable(Unmeasurable::TooSmall)
        );
        assert!(!detail.looks_upscaled());
    }

    #[test]
    fn a_flat_image_is_not_measurable() {
        let flat = RgbImage::from_fn(2560, 1440, |x, _| {
            let v = (x * 255 / 2559) as u8;
            image::Rgb([v, v, v])
        });
        let detail = measure_image(&DynamicImage::ImageRgb8(flat));
        assert_eq!(
            detail.estimate,
            Estimate::NotMeasurable(Unmeasurable::LittleTexture)
        );
        assert_eq!(detail.detail_size(), None);
        assert!(!detail.looks_upscaled());
    }

    #[test]
    fn a_tiny_image_is_too_small() {
        let detail = measure_image(&DynamicImage::ImageRgb8(textured(800, 500)));
        assert_eq!(
            detail.estimate,
            Estimate::NotMeasurable(Unmeasurable::TooSmall)
        );
    }

    #[test]
    fn the_same_image_gives_the_same_answer() {
        let image = DynamicImage::ImageRgb8(fake(&textured(2560, 1440), 720));
        let first = measure_image(&image);
        for _ in 0..3 {
            assert_eq!(measure_image(&image), first);
        }
    }

    #[test]
    fn scaled_size_keeps_the_aspect() {
        assert_eq!(scaled_size(3840, 2160, 1080), (1920, 1080));
        assert_eq!(scaled_size(2160, 3840, 1440), (1440, 2560));
    }

    /// The D48 gate (Work Order item 39): measures camera originals scaled
    /// to 4K, synthetic fakes made from them (upscaled from 1080p and 1440p
    /// with several filters, plain, sharpened or JPEG-recompressed), the
    /// labelled candidates and a sample of the library, and writes one JSON
    /// line per measurement. Reads private images and writes only to the
    /// output file; development only, driven by `tools/detail-gate.py`.
    ///
    /// `SHARPR_DETAIL_MANIFEST` (JSON with `natives`, `labelled`, `sample`
    /// path lists) and `SHARPR_DETAIL_OUTPUT` (a `.jsonl` path) are required.
    #[test]
    #[ignore]
    fn detail_gate_report() {
        use std::io::Write;
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(env("SHARPR_DETAIL_MANIFEST")).unwrap()).unwrap();
        let list = |key: &str| -> Vec<String> {
            manifest[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut out = std::fs::File::create(env("SHARPR_DETAIL_OUTPUT")).unwrap();
        let mut write = |record: serde_json::Value| {
            writeln!(out, "{record}").unwrap();
            out.flush().unwrap();
        };
        // Losses at other probe factors, for tuning without another run.
        let probes = |image: &DynamicImage| -> serde_json::Value {
            if std::env::var_os("SHARPR_DETAIL_PROBES").is_some_and(|v| v == "0") {
                return serde_json::Value::Null;
            }
            let luma = image.to_luma32f();
            let (w, h) = luma.dimensions();
            [1.15, 1.25, 1.4]
                .iter()
                .map(|&p| {
                    (
                        p.to_string(),
                        serde_json::json!(measure_luma_probed(&luma, w, h, p).losses),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
                .into()
        };
        let record = |detail: &Detail, seconds: f64| {
            serde_json::json!({
                "width": detail.width,
                "height": detail.height,
                "estimate": match detail.estimate {
                    Estimate::Full => "full".to_string(),
                    Estimate::Below { short_side } => format!("below:{short_side}"),
                    Estimate::NotMeasurable(Unmeasurable::TooSmall) => "too-small".to_string(),
                    Estimate::NotMeasurable(Unmeasurable::LittleTexture) => "little-texture".to_string(),
                },
                "upscaled": detail.looks_upscaled(),
                "detail": detail.detail_size(),
                "words": describe(detail),
                "tiles": detail.textured_tiles,
                "losses": detail.losses,
                "focus": detail.focus,
                "seconds": seconds,
            })
        };
        let filters = [
            ("triangle", FilterType::Triangle),
            ("catmullrom", FilterType::CatmullRom),
            ("lanczos3", FilterType::Lanczos3),
        ];
        for (n, path) in list("natives").iter().enumerate() {
            let Ok(decoded) = crate::export::decode_oriented(Path::new(path)) else {
                write(serde_json::json!({"set": "native", "path": path, "error": "decode"}));
                continue;
            };
            let mut native = decoded.to_rgb8();
            drop(decoded);
            let original_short = native.width().min(native.height());
            if native.width().min(native.height()) > 2160 {
                let (w, h) = scaled_size(native.width(), native.height(), 2160);
                native = imageops::resize(&native, w, h, FilterType::Lanczos3);
            }
            let started = std::time::Instant::now();
            let image = DynamicImage::ImageRgb8(native.clone());
            let detail = measure_image(&image);
            let mut r = record(&detail, started.elapsed().as_secs_f64());
            r["probes"] = probes(&image);
            r["source_short_side"] = original_short.into();
            r["set"] = "native".into();
            r["path"] = path.as_str().into();
            write(r);
            for source in [1080u32, 1440] {
                let (sw, sh) = scaled_size(native.width(), native.height(), source);
                let small = imageops::resize(&native, sw, sh, FilterType::Lanczos3);
                for (filter_name, filter) in filters {
                    let up = imageops::resize(&small, native.width(), native.height(), filter);
                    let quality = [85u8, 90, 95][n % 3];
                    let treatments: [(&str, RgbImage); 3] = [
                        ("plain", up.clone()),
                        ("sharpened", imageops::unsharpen(&up, 1.2, 1)),
                        ("jpeg", {
                            let mut bytes = Vec::new();
                            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
                                .encode_image(&up)
                                .unwrap();
                            image::load_from_memory(&bytes).unwrap().to_rgb8()
                        }),
                    ];
                    for (treatment, image) in treatments {
                        let started = std::time::Instant::now();
                        let image = DynamicImage::ImageRgb8(image);
                        let detail = measure_image(&image);
                        let mut r = record(&detail, started.elapsed().as_secs_f64());
                        r["probes"] = probes(&image);
                        r["set"] = "fake".into();
                        r["path"] = path.as_str().into();
                        r["source"] = source.into();
                        r["filter"] = filter_name.into();
                        r["treatment"] = treatment.into();
                        if treatment == "jpeg" {
                            r["quality"] = quality.into();
                        }
                        write(r);
                    }
                }
            }
        }
        for set in ["labelled", "sample"] {
            for path in list(set) {
                let started = std::time::Instant::now();
                let mut r = match measure_path(Path::new(&path)) {
                    Ok(detail) => record(&detail, started.elapsed().as_secs_f64()),
                    Err(error) => serde_json::json!({"error": error}),
                };
                r["set"] = set.into();
                r["path"] = path.as_str().into();
                write(r);
            }
        }
    }

    /// Labelled fake-4K candidates for the D48 gate, found without the
    /// estimator: images of at least 3840×2160 that have a clearly smaller
    /// version elsewhere in the library (Show Similar's Exact rule on the
    /// stored vectors). Opens the real index and vector file read-only and
    /// writes JSON to `SHARPR_DETAIL_CANDIDATES`; development only.
    #[test]
    #[ignore]
    fn detail_labelled_candidates() {
        use rayon::prelude::*;
        use rusqlite::{Connection, OpenFlags};
        use std::collections::HashMap;
        let output =
            std::env::var("SHARPR_DETAIL_CANDIDATES").expect("set SHARPR_DETAIL_CANDIDATES");
        let data = dirs::data_dir().unwrap().join("sharpr");
        let open = |name: &str| {
            Connection::open_with_flags(data.join(name), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
        };
        let index = open("library-index.sqlite");
        let mut sizes: HashMap<String, (u32, u32)> = HashMap::new();
        let mut stmt = index
            .prepare("SELECT path, width, height FROM images WHERE width > 0 AND height > 0")
            .unwrap();
        for row in stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            })
            .unwrap()
        {
            let (path, w, h) = row.unwrap();
            sizes.insert(path, (w, h));
        }
        let mut images: Vec<(String, String, (u32, u32))> = Vec::new();
        let mut stmt = index
            .prepare("SELECT path, hash FROM file_fingerprints")
            .unwrap();
        for row in stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
        {
            let (path, hash) = row.unwrap();
            if let Some(&size) = sizes.get(&path) {
                if Path::new(&path).exists() {
                    images.push((path, hash, size));
                }
            }
        }
        let store = open("visual-search.sqlite");
        let model: String = store
            .query_row(
                "SELECT model FROM vectors GROUP BY model ORDER BY count(*) DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
        let mut stmt = store
            .prepare("SELECT fingerprint, vector FROM vectors WHERE model = ?1")
            .unwrap();
        for row in stmt
            .query_map([&model], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })
            .unwrap()
        {
            let (fp, blob) = row.unwrap();
            vectors.insert(
                fp,
                blob.chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect(),
            );
        }
        let short = |(w, h): (u32, u32)| w.min(h);
        let targets: Vec<&(String, String, (u32, u32))> = images
            .iter()
            .filter(|(_, fp, (w, h))| {
                (*w).max(*h) >= 3840 && (*w).min(*h) >= 2160 && vectors.contains_key(fp)
            })
            .collect();
        let readers = crate::copies::Readers {
            size_of: &|_| None,
            tiny: &crate::copies::tiny_version,
            canceled: &|| false,
            progress: &|_, _| {},
        };
        let found: Vec<serde_json::Value> = targets
            .par_iter()
            .filter_map(|(path, fp, size)| {
                let va = &vectors[fp];
                type Image = (String, String, (u32, u32));
                let mut best: Option<(f32, &Image)> = None;
                for other in &images {
                    if other.1 == *fp || f64::from(short(other.2)) > 0.8 * f64::from(short(*size)) {
                        continue;
                    }
                    let Some(vb) = vectors.get(&other.1) else {
                        continue;
                    };
                    let score = crate::visual_search::similarity(va, vb);
                    if score >= crate::copies::MAYBE_SAME && best.is_none_or(|(b, _)| score > b) {
                        best = Some((score, other));
                    }
                }
                let (score, other) = best?;
                crate::copies::version_of(
                    (Path::new(path), fp),
                    (Path::new(&other.0), &other.1),
                    score,
                    &readers,
                )
                .then(|| {
                    serde_json::json!({
                        "path": path, "size": size,
                        "source": other.0, "source_size": other.2,
                        "similarity": score,
                    })
                })
            })
            .collect();
        std::fs::write(output, serde_json::to_vec_pretty(&found).unwrap()).unwrap();
    }
}
