//! Show Similar (Work Order item 28, D44): the images ranked by how close
//! their stored vectors are to one image's own, with no model run.
//!
//! Ranked, not cut at a score (user's library, October 19, 2026): scores
//! bunch high (half of all pairs above 0.68), and ordinary similar pictures
//! sit at 0.83–0.87 while the AI series first used to calibrate reach 0.94
//! and more, so no single cutoff finds both. The order is what holds up.
//! The levels nest: Exact ⊂ Close ⊂ Similar.

use std::path::PathBuf;

use super::search::Vectors;
use super::similarity;

/// Close always lists at least this many of the nearest pictures, so it is
/// never empty while Similar isn't. For a picture like nothing in
/// particular (the user's lava wallpaper) no neighbour stands out.
pub const CLOSE_AT_LEAST: usize = 12;

/// How far above an image's average similarity, in standard deviations, a
/// picture must be to stand out as close. In the user's library this kept
/// whole series (39 renders of one prompt, 73 shots of one outing) and a
/// handful for single pictures.
pub const STANDS_OUT: f32 = 3.0;

/// Which images Show Similar lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    /// Versions of the image: copies, and resized, converted, cropped or
    /// lightly edited ones (`copies::version_of`).
    Exact,
    /// The versions, the `CLOSE_AT_LEAST` nearest pictures, and every
    /// picture that stands out as close (`close_cutoff`).
    Close,
    /// Everything else, closest first.
    #[default]
    Similar,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Exact, Level::Close, Level::Similar];

    pub fn label(self) -> &'static str {
        match self {
            Level::Exact => "Exact",
            Level::Close => "Close",
            Level::Similar => "Similar",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Level::Exact => "This picture again: copies, and resized, cropped or edited versions",
            Level::Close => "The nearest pictures, and whole series that stand out",
            Level::Similar => "Pictures that look like this one, closest first",
        }
    }
}

/// The score from which a picture stands out as close to the image with
/// fingerprint `target`: `STANDS_OUT` standard deviations above its average
/// similarity to every other indexed picture in the library, whatever the
/// scope, so Close means the same in a folder as in the library.
pub fn close_cutoff(target: &str, vectors: &Vectors) -> Option<f32> {
    let query = vectors.get(target)?;
    let scores: Vec<f32> = vectors
        .iter()
        .filter(|(fingerprint, _)| fingerprint.as_str() != target)
        .map(|(_, vector)| similarity(query, vector))
        .collect();
    if scores.is_empty() {
        return None;
    }
    let n = scores.len() as f32;
    let mean = scores.iter().sum::<f32>() / n;
    let spread = (scores.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / n).sqrt();
    Some(mean + STANDS_OUT * spread)
}

/// Whether the `rank`th nearest other picture (from 0, versions not
/// counted) belongs in Close.
pub fn is_close(rank: usize, score: f32, cutoff: f32) -> bool {
    rank < CLOSE_AT_LEAST || score >= cutoff
}

/// One candidate: its path, fingerprint and similarity to the image.
#[derive(Clone, Debug, PartialEq)]
pub struct Neighbour {
    pub path: PathBuf,
    pub fingerprint: String,
    pub score: f32,
}

/// The candidates with a vector, closest to the image with fingerprint
/// `target` first, and how many had a vector. `None` when the image has no
/// vector yet. Equally close candidates keep the order they came in.
pub fn neighbours(
    target: &str,
    candidates: &[(PathBuf, Option<String>)],
    vectors: &Vectors,
) -> Option<Vec<Neighbour>> {
    let query = vectors.get(target)?;
    let mut ranked: Vec<Neighbour> = candidates
        .iter()
        .filter_map(|(path, fingerprint)| {
            let fingerprint = fingerprint.as_ref()?;
            let vector = vectors.get(fingerprint)?;
            Some(Neighbour {
                path: path.clone(),
                fingerprint: fingerprint.clone(),
                score: similarity(query, vector),
            })
        })
        .collect();
    // A stable sort: ties stay in candidate order.
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    Some(ranked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A unit vector at `degrees` in the first two dimensions; the cosine
    /// of two of them is the cosine of the angle between.
    fn at(degrees: f32) -> Vec<f32> {
        let mut v = vec![0.0; 4];
        v[0] = degrees.to_radians().cos();
        v[1] = degrees.to_radians().sin();
        v
    }

    #[test]
    fn neighbours_come_closest_first_and_ties_keep_their_order() {
        let vectors: Vectors = Arc::new(HashMap::from([
            ("shot".to_string(), at(0.0)),
            ("retake".to_string(), at(12.0)),
            ("lava".to_string(), at(32.0)),
            ("other".to_string(), at(60.0)),
        ]));
        let list: Vec<(PathBuf, Option<String>)> = [
            ("/l/other.jpg", Some("other")),
            ("/l/lava.jpg", Some("lava")),
            ("/l/b-copy.jpg", Some("shot")),
            ("/l/shot.jpg", Some("shot")),
            ("/l/retake.jpg", Some("retake")),
            ("/l/new.jpg", None),
        ]
        .iter()
        .map(|(p, f)| (PathBuf::from(p), f.map(str::to_string)))
        .collect();
        let ranked = neighbours("shot", &list, &vectors).unwrap();
        let names: Vec<_> = ranked
            .iter()
            .map(|n| n.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "/l/b-copy.jpg",
                "/l/shot.jpg",
                "/l/retake.jpg",
                "/l/lava.jpg",
                "/l/other.jpg"
            ]
        );
        assert!(neighbours("unindexed", &list, &vectors).is_none());
    }

    /// Close keeps the nearest few whatever their score, and beyond them
    /// only pictures that stand out from the image's usual similarity.
    #[test]
    fn close_keeps_the_nearest_and_whatever_stands_out() {
        // One target, a burst of 3 close takes, and 40 unrelated pictures
        // spread around 70°–90°.
        let mut map = HashMap::from([("shot".to_string(), at(0.0))]);
        for (k, degrees) in [10.0, 11.0, 12.0].into_iter().enumerate() {
            map.insert(format!("take{k}"), at(degrees));
        }
        for k in 0..40 {
            map.insert(format!("other{k}"), at(70.0 + k as f32 / 2.0));
        }
        let vectors: Vectors = Arc::new(map);
        let cutoff = close_cutoff("shot", &vectors).unwrap();
        assert!(
            similarity(&at(0.0), &at(12.0)) >= cutoff,
            "the burst stands out"
        );
        assert!(similarity(&at(0.0), &at(70.0)) < cutoff, "the rest doesn't");
        assert!(is_close(CLOSE_AT_LEAST - 1, 0.1, cutoff), "the nearest few");
        assert!(!is_close(CLOSE_AT_LEAST, 0.1, cutoff));
        assert!(
            is_close(CLOSE_AT_LEAST + 20, 0.99, cutoff),
            "a series beyond them"
        );
        assert!(close_cutoff("unindexed", &vectors).is_none());
    }
}
