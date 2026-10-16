//! Finding copies to clean up (`ROADMAP.md` Phase 7, decision D14): exact
//! copies, and resized or converted copies of the same picture. Crops and
//! merely similar pictures are not copies. GTK-free; runs on a worker.
//!
//! - **Exact copies** share a content fingerprint (Phase 4).
//! - **Resized or converted copies** are found through the visual-search
//!   vectors (Phase 5), which barely change with size or format: a cosine
//!   of 0.95 or more makes a candidate. A candidate also needs the same
//!   aspect ratio, since a crop changes it and copies keep it (without
//!   that, a crop could chain into a group through a close copy), and
//!   32×32 colour versions that look the same. The pixel check applies to
//!   every candidate: a real library had over a thousand pairs above 0.99,
//!   too many to trust the score alone with files headed for the trash.
//!   The small versions come from the thumbnail cache where possible, and
//!   each image is shrunk once.
//! - Matches can chain (A like B, B like C), so every member of a group is
//!   finally checked directly against the copy to keep, the one with the
//!   most pixels; one that doesn't match it leaves the group.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::library_index::LibraryIndex;

/// At or above this cosine, two images may be the same picture; the aspect
/// and pixel checks decide.
pub const MAYBE_SAME: f32 = 0.95;
/// The largest mean difference, out of 255, between 32×32 colour versions
/// of two images that still counts as the same picture.
const PIXEL_TOLERANCE: f64 = 10.0;

#[derive(Clone, Debug, PartialEq)]
pub struct CopyFile {
    pub path: PathBuf,
    /// Displayed (oriented) size, if it could be read.
    pub size: Option<(u32, u32)>,
    pub bytes: u64,
    /// Size and modification time when found (`fingerprint::file_stamp`),
    /// checked again before the file is trashed.
    pub stamp: (u64, i64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopyGroup {
    /// Highest resolution first; the first is the one to keep.
    pub files: Vec<CopyFile>,
    /// Every file has the same bytes.
    pub identical: bool,
}

impl CopyGroup {
    /// Bytes the extras take up.
    pub fn extra_bytes(&self) -> u64 {
        self.files.iter().skip(1).map(|f| f.bytes).sum()
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct CopyReport {
    pub groups: Vec<CopyGroup>,
    /// Images that have no visual-search vector yet, so only exact copies
    /// of them could be found.
    pub not_indexed: usize,
    pub images: usize,
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut node = i;
        while self.0[node] != root {
            let next = self.0[node];
            self.0[node] = root;
            node = next;
        }
        root
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[b] = a;
        }
    }
}

/// Whether two sizes have the same aspect ratio within 2%.
fn same_aspect(a: (u32, u32), b: (u32, u32)) -> bool {
    let ratio = |(w, h): (u32, u32)| f64::from(w) / f64::from(h.max(1));
    (ratio(a) / ratio(b) - 1.0).abs() <= 0.02
}

/// A 32×32 colour version of `path`, upright, for the pixel check: from a
/// cached thumbnail when there is one, which is far cheaper than decoding.
pub fn tiny_version(path: &Path) -> Option<Vec<u8>> {
    let image = match crate::thumbnails::worker::load_cached_rgba(path) {
        Some((rgba, width, height)) => {
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(width, height, rgba)?)
        }
        None => {
            let png = crate::visual_search::prepare::prepare_png(path).ok()?;
            image::load_from_memory(&png).ok()?
        }
    };
    Some(
        image
            .resize_exact(32, 32, image::imageops::FilterType::Triangle)
            .to_rgb8()
            .into_raw(),
    )
}

/// Whether two 32×32 versions show the same picture.
fn look_the_same(a: &[u8], b: &[u8]) -> bool {
    let diff: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(x.abs_diff(*y)))
        .sum::<f64>()
        / a.len().max(1) as f64;
    a.len() == b.len() && diff <= PIXEL_TOLERANCE
}

/// How to read the files `group` checks; tests substitute their own.
pub struct Readers<'a> {
    /// Displayed (oriented) size.
    pub size_of: &'a (dyn Fn(&Path) -> Option<(u32, u32)> + Sync),
    /// A 32×32 version, as from `tiny_version`.
    pub tiny: &'a (dyn Fn(&Path) -> Option<Vec<u8>> + Sync),
    pub canceled: &'a (dyn Fn() -> bool + Sync),
    /// (images checked, images to check) during the pixel checks.
    pub progress: &'a (dyn Fn(usize, usize) + Sync),
}

/// Groups the images in `fingerprints` (path → fingerprint, current files
/// only) into copies, using `vectors` (fingerprint → unit vector) for
/// resized and converted copies. `None` when canceled.
pub fn group(
    fingerprints: &BTreeMap<PathBuf, String>,
    vectors: &HashMap<String, Vec<f32>>,
    read: &Readers,
) -> Option<Vec<Vec<PathBuf>>> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    // One node per distinct fingerprint, with every file that has it.
    let mut by_hash: BTreeMap<&str, Vec<&PathBuf>> = BTreeMap::new();
    for (path, hash) in fingerprints {
        by_hash.entry(hash).or_default().push(path);
    }
    let nodes: Vec<(&str, Vec<&PathBuf>)> = by_hash.into_iter().collect();
    let first = |i: usize| nodes[i].1[0].as_path();
    let indexed: Vec<(usize, &Vec<f32>)> = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, (hash, _))| vectors.get(*hash).map(|v| (i, v)))
        .collect();
    // Every pair of distinct pictures, in parallel rows.
    let pairs: Vec<(usize, usize)> = (0..indexed.len())
        .into_par_iter()
        .flat_map_iter(|a| {
            let (i, va) = indexed[a];
            indexed[a + 1..].iter().filter_map(move |(j, vb)| {
                (crate::visual_search::similarity(va, vb) >= MAYBE_SAME).then_some((i, *j))
            })
        })
        .collect();
    if (read.canceled)() {
        return None;
    }
    // Each image's size and small version, read once and in parallel.
    let mut involved: Vec<usize> = pairs.iter().flat_map(|&(i, j)| [i, j]).collect();
    involved.sort_unstable();
    involved.dedup();
    let sizes: HashMap<usize, Option<(u32, u32)>> = involved
        .par_iter()
        .map(|&i| (i, (read.size_of)(first(i))))
        .collect();
    let candidates: Vec<(usize, usize)> = pairs
        .into_iter()
        .filter(|(i, j)| match (sizes[i], sizes[j]) {
            (Some(a), Some(b)) => same_aspect(a, b),
            _ => false,
        })
        .collect();
    let mut to_shrink: Vec<usize> = candidates.iter().flat_map(|&(i, j)| [i, j]).collect();
    to_shrink.sort_unstable();
    to_shrink.dedup();
    let done = AtomicUsize::new(0);
    (read.progress)(0, to_shrink.len());
    let tiny: HashMap<usize, Option<Vec<u8>>> = to_shrink
        .par_iter()
        .map(|&i| {
            if (read.canceled)() {
                return (i, None);
            }
            let small = (read.tiny)(first(i));
            (read.progress)(done.fetch_add(1, Ordering::Relaxed) + 1, to_shrink.len());
            (i, small)
        })
        .collect();
    if (read.canceled)() {
        return None;
    }
    let mut sets = UnionFind::new(nodes.len());
    for (i, j) in candidates {
        if let (Some(a), Some(b)) = (&tiny[&i], &tiny[&j]) {
            if look_the_same(a, b) {
                sets.union(i, j);
            }
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..nodes.len() {
        components.entry(sets.find(i)).or_default().push(i);
    }
    // Each member must match the copy to keep directly, not only through
    // a chain; one that doesn't stands alone (with its exact copies).
    let pixels = |i: &usize| {
        sizes
            .get(i)
            .copied()
            .flatten()
            .map_or(0, |(w, h)| u64::from(w) * u64::from(h))
    };
    let mut groups: BTreeMap<usize, Vec<PathBuf>> = BTreeMap::new();
    for members in components.into_values() {
        // As the review window orders them: most pixels, then the bigger
        // file, then the first.
        let bytes = |i: usize| std::fs::metadata(first(i)).map_or(0, |m| m.len());
        let keeper = *members
            .iter()
            .max_by_key(|&&i| (pixels(&i), bytes(i), std::cmp::Reverse(i)))
            .expect("not empty");
        for i in members {
            let with_keeper = i == keeper
                || match (
                    &tiny.get(&i),
                    &tiny.get(&keeper),
                    sizes.get(&i),
                    sizes.get(&keeper),
                ) {
                    (Some(Some(a)), Some(Some(b)), Some(Some(sa)), Some(Some(sb))) => {
                        look_the_same(a, b) && same_aspect(*sa, *sb)
                    }
                    _ => false,
                };
            let group = if with_keeper { keeper } else { i };
            groups
                .entry(group)
                .or_default()
                .extend(nodes[i].1.iter().map(|p| (*p).clone()));
        }
    }
    Some(
        groups
            .into_values()
            .filter(|paths| paths.len() > 1)
            .collect(),
    )
}

/// How many other existing files have exactly the bytes of `path`, from
/// cached fingerprints only. Only in-scope file stamps are checked; image
/// bytes are never read or hashed here.
pub fn identical_elsewhere(
    index: &LibraryIndex,
    path: &Path,
    root: Option<&Path>,
    disabled: &[PathBuf],
) -> usize {
    if !crate::buckets::visible(path, root, disabled) {
        return 0;
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return 0;
    };
    let (size, mtime) = crate::fingerprint::file_stamp(&metadata);
    let Ok(Some(hash)) = index.fresh_fingerprint(path, size, mtime) else {
        return 0;
    };
    let Ok(files) = index.files_with_fingerprints(std::slice::from_ref(&hash)) else {
        return 0;
    };
    files
        .get(&hash)
        .into_iter()
        .flatten()
        .filter(|(other, size, mtime)| {
            other != path
                && crate::buckets::visible(other, root, disabled)
                && std::fs::metadata(other)
                    .is_ok_and(|m| crate::fingerprint::file_stamp(&m) == (*size, *mtime))
        })
        .count()
}

/// Finds every group of copies among `paths` (the images in view), using
/// the visual-search vectors in `vectors` (empty without the model).
pub fn find(
    index: &LibraryIndex,
    paths: &[PathBuf],
    vectors: &HashMap<String, Vec<f32>>,
    canceled: &(dyn Fn() -> bool + Sync),
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<CopyReport, String> {
    let cached = index.cached_fingerprints().map_err(|e| e.to_string())?;
    let mut fingerprints = BTreeMap::new();
    let mut stamps = HashMap::new();
    for path in paths {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        let stamp = crate::fingerprint::file_stamp(&metadata);
        if let Some((size, mtime, hash)) = cached.get(path.to_string_lossy().as_ref()) {
            if (*size, *mtime) == stamp {
                fingerprints.insert(path.clone(), hash.clone());
                stamps.insert(path.clone(), stamp);
            }
        }
    }
    let not_indexed = fingerprints
        .values()
        .filter(|hash| !vectors.contains_key(*hash))
        .count();
    let size_of = |path: &Path| crate::export::oriented_source_dimensions(path).ok();
    let read = Readers {
        size_of: &size_of,
        tiny: &tiny_version,
        canceled,
        progress,
    };
    let mut groups: Vec<CopyGroup> = group(&fingerprints, vectors, &read)
        .ok_or("Canceled")?
        .into_iter()
        .map(|paths| {
            let identical = paths
                .iter()
                .all(|p| fingerprints.get(p) == fingerprints.get(&paths[0]));
            let mut files: Vec<CopyFile> = paths
                .into_iter()
                .map(|path| {
                    let stamp = stamps.get(&path).copied().unwrap_or_default();
                    CopyFile {
                        size: size_of(&path),
                        bytes: stamp.0,
                        stamp,
                        path,
                    }
                })
                .collect();
            // Keep the most pixels; then the bigger file (less compressed);
            // then the shorter, earlier path.
            files.sort_by(|a, b| {
                let pixels = |f: &CopyFile| f.size.map_or(0, |(w, h)| u64::from(w) * u64::from(h));
                pixels(b)
                    .cmp(&pixels(a))
                    .then(b.bytes.cmp(&a.bytes))
                    .then(a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
                    .then(a.path.cmp(&b.path))
            });
            CopyGroup { files, identical }
        })
        .collect();
    groups.sort_by_key(|group| std::cmp::Reverse(group.extra_bytes()));
    Ok(CopyReport {
        groups,
        not_indexed,
        images: paths.len(),
    })
}

/// One group's part of a confirmed trash request: the ticked files and
/// the ones left unticked, each with the stamp it had when reviewed.
#[derive(Clone, Debug, Default)]
pub struct TrashGroup {
    pub extras: Vec<(PathBuf, (u64, i64))>,
    pub kept: Vec<(PathBuf, (u64, i64))>,
}

/// What to do with each ticked file of a confirmed request.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Trash(PathBuf),
    /// Left alone, with the reason.
    Skip(PathBuf, &'static str),
}

fn unchanged(path: &Path, stamp: (u64, i64)) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|m| m.is_file() && crate::fingerprint::file_stamp(&m) == stamp)
}

/// Checks a confirmed request against the files as they are now, just
/// before trashing: a file that changed since the review (replaced,
/// edited, or gone) stays, and a group whose kept copies are all gone or
/// changed keeps its extras too, unless the user ticked the whole group.
pub fn verify(group: &TrashGroup) -> Vec<Verdict> {
    let keeper_ok = group.kept.is_empty()
        || group
            .kept
            .iter()
            .any(|(path, stamp)| unchanged(path, *stamp));
    group
        .extras
        .iter()
        .map(|(path, stamp)| {
            if !keeper_ok {
                Verdict::Skip(path.clone(), "the copy to keep changed or is gone")
            } else if !unchanged(path, *stamp) {
                Verdict::Skip(path.clone(), "it changed since the review")
            } else {
                Verdict::Trash(path.clone())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {

    #[test]
    fn info_copy_counts_only_include_the_enabled_root() {
        let root = std::env::temp_dir().join(format!(
            "sharpr-copy-scope-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let active = root.join("active");
        let disabled = active.join("disabled");
        std::fs::create_dir_all(&disabled).unwrap();
        let source = active.join("a.png");
        let allowed = active.join("b.png");
        let hidden = disabled.join("c.png");
        let outside = root.join("d.png");
        for path in [&source, &allowed, &hidden, &outside] {
            std::fs::write(path, b"same image bytes").unwrap();
        }
        let index = crate::library_index::LibraryIndex::open_in_memory().unwrap();
        crate::fingerprint::run_pass(&index, &root, &[], &[], &|| false, &|_| {}).unwrap();
        assert_eq!(identical_elsewhere(&index, &source, None, &[]), 3);
        assert_eq!(
            identical_elsewhere(
                &index,
                &source,
                Some(&active),
                std::slice::from_ref(&disabled)
            ),
            1
        );
        assert_eq!(
            identical_elsewhere(
                &index,
                &hidden,
                Some(&active),
                std::slice::from_ref(&disabled)
            ),
            0
        );
        assert_eq!(
            identical_elsewhere(
                &index,
                &outside,
                Some(&active),
                std::slice::from_ref(&disabled)
            ),
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    use super::*;

    fn unit(values: &[(usize, f32)]) -> Vec<f32> {
        let mut v = vec![0.0f32; crate::visual_search::DIMENSIONS];
        for (i, x) in values {
            v[*i] = *x;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / norm).collect()
    }

    #[test]
    fn copies_group_by_fingerprint_and_by_vector_but_crops_do_not() {
        let p = |name: &str| PathBuf::from(format!("/l/{name}"));
        let fingerprints: BTreeMap<PathBuf, String> = [
            (p("a.png"), "A"),
            (p("a-copy.png"), "A"),   // identical bytes
            (p("a-1080p.jpg"), "A2"), // resized: same picture
            (p("a.webp"), "A3"),      // converted, borderline score
            (p("a-crop.png"), "A4"),  // a crop, borderline score
            (p("other.jpg"), "B"),    // a different picture
            (p("lonely.png"), "C"),   // no vector at all
        ]
        .into_iter()
        .map(|(path, hash)| (path, hash.to_string()))
        .collect();
        let vectors: HashMap<String, Vec<f32>> = [
            ("A", unit(&[(0, 1.0)])),
            ("A2", unit(&[(0, 1.0), (1, 0.05)])), // cosine ≈ 0.9988, still checked
            ("A3", unit(&[(0, 1.0), (1, 0.2)])),  // cosine ≈ 0.98
            ("A4", unit(&[(0, 1.0), (1, 0.25)])), // cosine ≈ 0.97
            ("B", unit(&[(2, 1.0)])),
        ]
        .into_iter()
        .map(|(h, v)| (h.to_string(), v))
        .collect();
        let size_of = |path: &Path| {
            Some(if path.ends_with("a-crop.png") {
                (1000, 1000)
            } else {
                (1920, 1080)
            })
        };
        let read = |tiny: &(dyn Fn(&Path) -> Option<Vec<u8>> + Sync)| {
            group(
                &fingerprints,
                &vectors,
                &Readers {
                    size_of: &size_of,
                    tiny,
                    canceled: &|| false,
                    progress: &|_, _| {},
                },
            )
            .unwrap()
        };
        let all_alike = |_: &Path| Some(vec![128u8; 1024]);
        let groups = read(&all_alike);
        assert_eq!(groups.len(), 1, "{groups:?}");
        let mut members = groups[0].clone();
        members.sort();
        assert_eq!(
            members,
            vec![p("a-1080p.jpg"), p("a-copy.png"), p("a.png"), p("a.webp")],
            "the crop's aspect differs, so it stays out"
        );

        // Pixels that differ keep even a very close score apart; exact
        // copies need no pixel check.
        let webp_differs =
            |path: &Path| Some(vec![if path.ends_with("a.webp") { 0u8 } else { 128 }; 1024]);
        let groups = read(&webp_differs);
        let mut members = groups[0].clone();
        members.sort();
        assert_eq!(members, vec![p("a-1080p.jpg"), p("a-copy.png"), p("a.png")]);
        let unreadable = |path: &Path| (!path.ends_with("a-1080p.jpg")).then(|| vec![128u8; 1024]);
        let groups = read(&unreadable);
        let mut members = groups[0].clone();
        members.sort();
        assert_eq!(members, vec![p("a-copy.png"), p("a.png"), p("a.webp")]);

        // Canceling returns nothing.
        let canceled = group(
            &fingerprints,
            &vectors,
            &Readers {
                size_of: &size_of,
                tiny: &all_alike,
                canceled: &|| true,
                progress: &|_, _| {},
            },
        );
        assert!(canceled.is_none());

        // A chain: the 1080p copy is like both the original and the WebP,
        // but the WebP doesn't match the original directly, so it leaves.
        let chained = |path: &Path| {
            let shade = if path.ends_with("a.webp") {
                144u8
            } else if path.ends_with("a-1080p.jpg") {
                136
            } else {
                128
            };
            Some(vec![shade; 1024])
        };
        let groups = read(&chained);
        let mut members = groups[0].clone();
        members.sort();
        assert_eq!(members, vec![p("a-1080p.jpg"), p("a-copy.png"), p("a.png")]);
    }

    #[test]
    fn trashing_skips_files_that_changed_and_groups_that_lost_their_keeper() {
        let dir = std::env::temp_dir().join(format!("sharpr-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            let stamp = crate::fingerprint::file_stamp(&std::fs::metadata(&path).unwrap());
            (path, stamp)
        };
        let keep = file("keep.png", b"keeper");
        let extra = file("extra.png", b"copy");
        let swapped = file("swapped.png", b"copy");
        let group = TrashGroup {
            extras: vec![extra.clone(), swapped.clone()],
            kept: vec![keep.clone()],
        };
        // Something else now sits where a reviewed extra was.
        std::fs::remove_file(&swapped.0).unwrap();
        std::fs::write(&swapped.0, b"an unrelated picture").unwrap();
        assert_eq!(
            verify(&group),
            vec![
                Verdict::Trash(extra.0.clone()),
                Verdict::Skip(swapped.0.clone(), "it changed since the review"),
            ]
        );
        // The kept copy went away during the review: nothing goes.
        std::fs::remove_file(&keep.0).unwrap();
        assert!(verify(&group)
            .iter()
            .all(|v| matches!(v, Verdict::Skip(_, "the copy to keep changed or is gone"))));
        // Unless the user ticked the whole group.
        let whole = TrashGroup {
            extras: vec![extra.clone()],
            kept: vec![],
        };
        assert_eq!(verify(&whole), vec![Verdict::Trash(extra.0.clone())]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn aspect_ratios_match_within_two_percent() {
        assert!(same_aspect((3840, 2160), (1920, 1080)));
        assert!(same_aspect((1920, 1080), (1920, 1090)));
        assert!(!same_aspect((1920, 1080), (1080, 1080)));
    }

    /// A busy picture: overlapping coloured discs on a gradient.
    fn scene(width: u32, height: u32) -> image::RgbImage {
        let mut seed = 0x5eed_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let discs: Vec<(f64, f64, f64, [u8; 3])> = (0..40)
            .map(|_| {
                (
                    f64::from(next() % 1000) / 1000.0,
                    f64::from(next() % 1000) / 1000.0,
                    0.03 + f64::from(next() % 100) / 1000.0,
                    [
                        (next() % 256) as u8,
                        (next() % 256) as u8,
                        (next() % 256) as u8,
                    ],
                )
            })
            .collect();
        image::RgbImage::from_fn(width, height, |x, y| {
            let (u, v) = (
                f64::from(x) / f64::from(width),
                f64::from(y) / f64::from(height),
            );
            let mut pixel = [(u * 120.0) as u8, (v * 160.0) as u8, 90];
            for (cx, cy, r, colour) in &discs {
                let dx = (u - cx) * f64::from(width) / f64::from(height);
                if dx * dx + (v - cy) * (v - cy) < r * r {
                    pixel = *colour;
                }
            }
            image::Rgb(pixel)
        })
    }

    /// The Phase 7 acceptance fixture with the real model: run with
    /// `SHARPR_TEST_MODEL_DIR` and `SHARPR_LLAMA_SERVER` (and optionally
    /// `SHARPR_LLAMA_SERVER_GPU`).
    #[test]
    #[ignore]
    fn the_real_model_groups_resized_and_converted_copies_but_not_crops() {
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let root = std::env::temp_dir().join(format!("sharpr-copies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let picture = image::DynamicImage::ImageRgb8(scene(3840, 2160));
        picture.save(root.join("wallpaper-4k.png")).unwrap();
        std::fs::copy(
            root.join("wallpaper-4k.png"),
            root.join("wallpaper-4k copy.png"),
        )
        .unwrap();
        picture
            .resize_exact(1920, 1080, image::imageops::FilterType::Lanczos3)
            .save(root.join("wallpaper-1080p.jpg"))
            .unwrap();
        std::fs::write(
            root.join("wallpaper.webp"),
            crate::export::webp_bytes(&picture, false, 80).unwrap(),
        )
        .unwrap();
        picture.fliph().save(root.join("mirrored.jpg")).unwrap();
        // A crop with the same 16:9 shape: only the vectors and pixels can
        // tell it apart.
        picture
            .crop_imm(0, 0, 1920, 1080)
            .save(root.join("crop-16x9.png"))
            .unwrap();

        let index = LibraryIndex::open_in_memory().unwrap();
        crate::fingerprint::run_pass(&index, &root, &[], &[], &|| false, &|_| {}).unwrap();
        let files = crate::visual_search::files::locate_in(
            Path::new(&env("SHARPR_TEST_MODEL_DIR")),
            &[PathBuf::from(env("SHARPR_LLAMA_SERVER"))],
        )
        .unwrap();
        let store = crate::visual_search::store::Store::open(&root.join("vectors.sqlite")).unwrap();
        let mut embedder = crate::visual_search::index::ServerEmbedder::new(
            files.clone(),
            files.gpu_server.is_some(),
            std::sync::Arc::new(std::sync::Mutex::new(None)),
        );
        crate::visual_search::index::run_pass(
            &index,
            &store,
            "m",
            &root,
            &[],
            &|| false,
            &|_| {},
            &mut embedder,
        )
        .unwrap();
        drop(embedder);
        let vectors = store.vectors("m").unwrap();

        let paths: Vec<PathBuf> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|e| ["png", "jpg", "webp"].contains(&e.to_str().unwrap()))
            })
            .collect();
        let report = find(&index, &paths, &vectors, &|| false, &|_, _| {}).unwrap();
        for group in &report.groups {
            eprintln!(
                "group: {:?}",
                group
                    .files
                    .iter()
                    .map(|f| f.path.file_name().unwrap())
                    .collect::<Vec<_>>()
            );
        }
        // Print the scores, to tune the thresholds.
        let cached = index.cached_fingerprints().unwrap();
        let vector_of = |name: &str| {
            let hash = &cached[root.join(name).to_str().unwrap()].2;
            vectors[hash].clone()
        };
        for name in [
            "wallpaper-1080p.jpg",
            "wallpaper.webp",
            "mirrored.jpg",
            "crop-16x9.png",
        ] {
            eprintln!(
                "{name}: {:.4}",
                crate::visual_search::similarity(&vector_of("wallpaper-4k.png"), &vector_of(name))
            );
        }
        let group = report
            .groups
            .iter()
            .find(|g| g.files.iter().any(|f| f.path.ends_with("wallpaper-4k.png")))
            .expect("the 4K picture is in a group");
        let names: Vec<String> = group
            .files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names[0], "wallpaper-4k.png",
            "the highest resolution comes first"
        );
        for copy in [
            "wallpaper-4k copy.png",
            "wallpaper-1080p.jpg",
            "wallpaper.webp",
        ] {
            assert!(
                names.contains(&copy.to_string()),
                "{copy} missing from {names:?}"
            );
        }
        assert!(
            !names.contains(&"crop-16x9.png".to_string()),
            "a crop is not a copy"
        );
        assert!(
            !names.contains(&"mirrored.jpg".to_string()),
            "a mirrored version is not a copy"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
