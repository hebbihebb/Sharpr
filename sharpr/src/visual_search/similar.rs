//! Show Similar (Work Order item 28, D44): the images ranked by how close
//! their stored vectors are to one image's own, with no model run.
//!
//! Ranked, not cut at a score (user's library, October 19, 2026): scores
//! bunch high (half of all pairs above 0.68), and ordinary similar pictures
//! sit at 0.83–0.87 while the AI series first used to calibrate reach 0.94
//! and more, so no single cutoff finds both. The order is what holds up.

use std::path::PathBuf;

use super::search::Vectors;
use super::similarity;

/// From here up, a picture that isn't a copy is another take of the same
/// scene: retakes, bursts and AI variants in the user's library.
pub const CLOSE_FROM: f32 = 0.94;

/// Which images Show Similar lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    /// Versions of the image: copies, and resized, converted, cropped or
    /// lightly edited ones (`copies::version_of`).
    Exact,
    /// Other takes of the same scene, at least `CLOSE_FROM` alike.
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
            Level::Close => "Other takes of the same scene",
            Level::Similar => "Pictures that look like this one, closest first",
        }
    }
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
        assert!(ranked[2].score >= CLOSE_FROM && ranked[3].score < CLOSE_FROM);
        assert!(neighbours("unindexed", &list, &vectors).is_none());
    }
}
