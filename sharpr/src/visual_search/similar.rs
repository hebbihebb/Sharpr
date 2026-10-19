//! Show Similar (Work Order item 28, D44): the images whose vectors are
//! close to one image's own, from the stored vectors with no model run.
//!
//! The thresholds come from the user's library (October 19, 2026): pairs
//! at 0.94 and above were series of one scene or shot in different takes;
//! from 0.96 the same picture re-rendered or lightly edited; at 0.92
//! different pictures that share a style began to join.

use std::path::PathBuf;

use super::search::Vectors;
use super::similarity;

/// How close an image must be to count as similar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Closeness {
    /// The same picture re-rendered or lightly edited.
    Close,
    /// One scene or shot in different takes.
    #[default]
    Series,
    /// Also pictures that only share a subject and style.
    Loose,
}

impl Closeness {
    pub const ALL: [Closeness; 3] = [Closeness::Close, Closeness::Series, Closeness::Loose];

    pub fn threshold(self) -> f32 {
        match self {
            Closeness::Close => 0.96,
            Closeness::Series => 0.94,
            Closeness::Loose => 0.92,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Closeness::Close => "Close",
            Closeness::Series => "Series",
            Closeness::Loose => "Loose",
        }
    }
}

/// What a Show Similar search found.
#[derive(Debug, PartialEq)]
pub struct Similar {
    /// The image itself first (with any identical copies), then the similar
    /// images, closest first.
    pub paths: Vec<PathBuf>,
    /// Similar images, not counting the image or its identical copies.
    pub similar: usize,
    /// Candidates with a vector, and all candidates.
    pub indexed: usize,
    pub eligible: usize,
}

/// The candidates at least `closeness` alike to the image with fingerprint
/// `target`, each candidate given with its current fingerprint if known.
/// `None` when the image has no vector yet. Equally close candidates keep
/// the order they came in.
pub fn similar_to(
    target: &str,
    candidates: &[(PathBuf, Option<String>)],
    vectors: &Vectors,
    closeness: Closeness,
) -> Option<Similar> {
    let query = vectors.get(target)?;
    let threshold = closeness.threshold();
    let mut same = Vec::new();
    let mut scored = Vec::new();
    let mut indexed = 0;
    for (path, fingerprint) in candidates {
        let Some(fingerprint) = fingerprint else {
            continue;
        };
        let Some(vector) = vectors.get(fingerprint) else {
            continue;
        };
        indexed += 1;
        if fingerprint == target {
            same.push(path.clone());
            continue;
        }
        let score = similarity(query, vector);
        if score >= threshold {
            scored.push((score, path));
        }
    }
    // A stable sort: ties stay in candidate order.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let similar = scored.len();
    same.extend(scored.into_iter().map(|(_, path)| path.clone()));
    Some(Similar {
        paths: same,
        similar,
        indexed,
        eligible: candidates.len(),
    })
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

    fn candidates(names: &[(&str, Option<&str>)]) -> Vec<(PathBuf, Option<String>)> {
        names
            .iter()
            .map(|(p, f)| (PathBuf::from(p), f.map(str::to_string)))
            .collect()
    }

    /// cos 12° ≈ 0.978, cos 18° ≈ 0.951, cos 22° ≈ 0.927, cos 40° ≈ 0.766.
    #[test]
    fn a_burst_is_found_and_other_pictures_are_left_out() {
        let vectors: Vectors = Arc::new(HashMap::from([
            ("shot".to_string(), at(0.0)),
            ("retake".to_string(), at(12.0)),
            ("series".to_string(), at(18.0)),
            ("style".to_string(), at(22.0)),
            ("other".to_string(), at(40.0)),
        ]));
        let list = candidates(&[
            ("/l/other.jpg", Some("other")),
            ("/l/style.jpg", Some("style")),
            ("/l/series.jpg", Some("series")),
            ("/l/shot.jpg", Some("shot")),
            ("/l/copy-of-shot.jpg", Some("shot")),
            ("/l/retake.jpg", Some("retake")),
            ("/l/new.jpg", None),
        ]);
        let found = |closeness| similar_to("shot", &list, &vectors, closeness).unwrap();
        let series = found(Closeness::Series);
        assert_eq!(
            series.paths,
            [
                "/l/shot.jpg",
                "/l/copy-of-shot.jpg",
                "/l/retake.jpg",
                "/l/series.jpg"
            ]
            .map(PathBuf::from)
        );
        assert_eq!((series.similar, series.indexed, series.eligible), (2, 6, 7));
        assert_eq!(found(Closeness::Close).similar, 1);
        assert_eq!(
            found(Closeness::Loose).paths.last(),
            Some(&PathBuf::from("/l/style.jpg"))
        );
        assert!(similar_to("unindexed", &list, &vectors, Closeness::Series).is_none());
    }
}
