//! Searching: rank images by how close they are to the typed words.
//!
//! A `Searcher` thread owns a text-only `llama-server` that it starts at the
//! first search and stops after `IDLE` without one. It turns words into a
//! query vector and keeps the indexed vectors in memory, reloading them when
//! indexing adds more. Ranking itself is a pure function over the candidates
//! that scope, disabled folders, and filter chips have already narrowed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use super::runtime::{Mode, Server};
use super::similarity;
use super::store::Store;

/// Results are shown this many at a time (`ROADMAP.md` Phase 5).
pub const PAGE: usize = 200;

#[cfg(not(test))]
const IDLE: Duration = Duration::from_secs(300);
#[cfg(test)]
const IDLE: Duration = Duration::from_millis(200);

/// Prompts that describe any image. An image's average closeness to them is
/// how close it is to every query, so subtracting it measures how
/// specifically an image matches (used for match strength only; the order
/// stays the plain closeness the user judged in Phase 3).
const GENERIC_PROMPTS: [&str; 10] = [
    "image",
    "picture",
    "photo",
    "wallpaper",
    "art",
    "something",
    "a thing",
    "scene",
    "background",
    "illustration",
];

/// Below this many indexed candidates, the fifth-best score says little
/// about the words, so strength isn't judged.
pub const JUDGE_FROM: usize = 50;

/// Indexed vectors by fingerprint, shared with every search that uses them.
pub type Vectors = Arc<HashMap<String, Vec<f32>>>;

/// What a search needs from the model, ready to rank with.
#[derive(Clone)]
pub struct Prepared {
    pub query: Vec<f32>,
    /// The query minus the generic prompts' mean.
    pub specific: Vec<f32>,
    pub vectors: Vectors,
}

/// How well the best results match, from the fifth-best specific score.
/// Calibrated on the judged Phase 3 sample: 28 of the 30 real queries
/// scored at least 0.028, and 13 of 15 subjects absent from the sample at
/// most 0.015.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Strong,
    Some,
    Weak,
}

impl Strength {
    fn from_score(score: f32) -> Self {
        if score >= 0.04 {
            Strength::Strong
        } else if score >= 0.02 {
            Strength::Some
        } else {
            Strength::Weak
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Ranked {
    /// The closest `limit` indexed candidates, closest first.
    pub paths: Vec<PathBuf>,
    /// Candidates with a vector; the rest aren't indexed yet.
    pub indexed: usize,
    pub eligible: usize,
    /// None when too few images are indexed to judge (`JUDGE_FROM`).
    pub strength: Option<Strength>,
}

/// Ranks `candidates` (each with its current fingerprint, if known) and
/// keeps the closest `limit`.
pub fn rank(candidates: &[(PathBuf, Option<String>)], prepared: &Prepared, limit: usize) -> Ranked {
    let mut scored: Vec<(f32, &PathBuf, &Vec<f32>)> = candidates
        .iter()
        .filter_map(|(path, fingerprint)| {
            let vector = prepared.vectors.get(fingerprint.as_ref()?)?;
            Some((similarity(&prepared.query, vector), path, vector))
        })
        .collect();
    let indexed = scored.len();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    let mut specific: Vec<f32> = scored
        .iter()
        .map(|(_, _, vector)| similarity(&prepared.specific, vector))
        .collect();
    specific.sort_by(|a, b| b.total_cmp(a));
    let strength = (indexed >= JUDGE_FROM).then(|| Strength::from_score(specific[4]));
    Ranked {
        paths: scored
            .into_iter()
            .take(limit)
            .map(|(_, path, _)| path.clone())
            .collect(),
        indexed,
        eligible: candidates.len(),
        strength,
    }
}

struct Request {
    text: String,
    reply: mpsc::Sender<Result<Prepared, String>>,
}

/// The search thread. Cheap to clone; all clones share the thread.
#[derive(Clone)]
pub struct Searcher {
    requests: mpsc::Sender<Request>,
}

impl Searcher {
    /// `vectors_version` changes whenever indexing may have added vectors.
    pub fn start(store_path: PathBuf, vectors_version: Arc<AtomicU64>) -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        let _ = std::thread::Builder::new()
            .name("visual-search".into())
            .spawn(move || {
                let mut state = State {
                    store_path,
                    vectors_version,
                    server: None,
                    generic: None,
                    model: None,
                    loaded: None,
                    last: None,
                };
                loop {
                    match rx.recv_timeout(IDLE) {
                        Ok(request) => {
                            let result = state.prepare(&request.text);
                            if result.is_err() {
                                state.server = None;
                            }
                            let _ = request.reply.send(result);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            // Idle: give the memory back.
                            state.server = None;
                            state.generic = None;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
        Self { requests: tx }
    }

    /// Blocks until the query is embedded. Errors explain why visual search
    /// can't run, in words for the user.
    pub fn prepare(&self, text: &str) -> Result<Prepared, String> {
        let (reply, response) = mpsc::channel();
        self.requests
            .send(Request {
                text: text.to_string(),
                reply,
            })
            .map_err(|_| "Visual search stopped".to_string())?;
        response
            .recv()
            .map_err(|_| "Visual search stopped".to_string())?
    }
}

struct State {
    store_path: PathBuf,
    vectors_version: Arc<AtomicU64>,
    server: Option<Server>,
    generic: Option<Vec<f32>>,
    /// The model key and files it was computed for.
    model: Option<(super::files::ModelFiles, String)>,
    /// Vectors for (model key, version).
    loaded: Option<(String, u64, Vectors)>,
    /// The last query, which "Show More" asks for again.
    last: Option<(String, Vec<f32>)>,
}

impl State {
    fn prepare(&mut self, text: &str) -> Result<Prepared, String> {
        let files = super::files::locate()?;
        let model = match &self.model {
            Some((known, key)) if *known == files => key.clone(),
            _ => {
                let key = Store::open(&self.store_path)?.model_key(&files)?;
                self.model = Some((files.clone(), key.clone()));
                self.server = None;
                self.last = None;
                key
            }
        };
        let version = self.vectors_version.load(Ordering::Relaxed);
        let vectors = match &self.loaded {
            Some((key, loaded_version, vectors)) if *key == model && *loaded_version == version => {
                vectors.clone()
            }
            _ => {
                let vectors = Arc::new(Store::open(&self.store_path)?.vectors(&model)?);
                self.loaded = Some((model.clone(), version, vectors.clone()));
                vectors
            }
        };
        if vectors.is_empty() {
            return Err("No images are indexed for visual search yet".into());
        }
        if self
            .server
            .as_mut()
            .is_some_and(|server| !server.is_alive())
        {
            self.server = None;
        }
        if self.server.is_none() {
            self.server = Some(Server::start(&files, Mode::Queries)?);
            self.generic = None;
        }
        let server = self.server.as_mut().expect("started above");
        if self.generic.is_none() {
            let mut mean = vec![0.0f32; super::DIMENSIONS];
            for prompt in GENERIC_PROMPTS {
                for (sum, value) in mean.iter_mut().zip(server.embed_query(prompt)?) {
                    *sum += value / GENERIC_PROMPTS.len() as f32;
                }
            }
            self.generic = Some(mean);
        }
        let query = match &self.last {
            Some((last, vector)) if last == text => vector.clone(),
            _ => {
                let vector = server.embed_query(text)?;
                self.last = Some((text.to_string(), vector.clone()));
                vector
            }
        };
        let generic = self.generic.as_ref().expect("computed above");
        let specific = query.iter().zip(generic).map(|(q, g)| q - g).collect();
        Ok(Prepared {
            query,
            specific,
            vectors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(index: usize, weight: f32) -> Vec<f32> {
        let mut v = vec![0.0; super::super::DIMENSIONS];
        v[index] = weight;
        v[767] = (1.0 - weight * weight).sqrt();
        v
    }

    #[test]
    fn ranking_orders_indexed_candidates_and_counts_coverage() {
        let vectors: HashMap<String, Vec<f32>> = [
            ("near".to_string(), axis(0, 0.9)),
            ("far".to_string(), axis(0, 0.2)),
            ("mid".to_string(), axis(0, 0.5)),
            ("outside".to_string(), axis(0, 1.0)),
        ]
        .into();
        let prepared = Prepared {
            query: axis(0, 1.0),
            specific: axis(0, 1.0),
            vectors: Arc::new(vectors),
        };
        let candidates = vec![
            (PathBuf::from("/a/far.jpg"), Some("far".to_string())),
            (PathBuf::from("/a/near.jpg"), Some("near".to_string())),
            (
                PathBuf::from("/a/copy-of-near.jpg"),
                Some("near".to_string()),
            ),
            (PathBuf::from("/a/mid.jpg"), Some("mid".to_string())),
            (PathBuf::from("/a/new.jpg"), None),
            (PathBuf::from("/a/unindexed.jpg"), Some("zz".to_string())),
        ];
        let ranked = rank(&candidates, &prepared, 3);
        assert_eq!(
            ranked.paths,
            vec![
                PathBuf::from("/a/copy-of-near.jpg"),
                PathBuf::from("/a/near.jpg"),
                PathBuf::from("/a/mid.jpg"),
            ],
            "closest first; an image outside the candidates never appears"
        );
        assert_eq!((ranked.indexed, ranked.eligible), (4, 6));
        assert_eq!(ranked.strength, None, "too few images to judge");

        let many: Vec<(PathBuf, Option<String>)> = (0..JUDGE_FROM)
            .map(|i| {
                (
                    PathBuf::from(format!("/b/{i}.jpg")),
                    Some("near".to_string()),
                )
            })
            .collect();
        assert_eq!(rank(&many, &prepared, 3).strength, Some(Strength::Strong));

        let none = rank(&candidates[4..], &prepared, 3);
        assert_eq!(
            (none.paths.len(), none.indexed, none.strength),
            (0, 0, None)
        );
    }

    #[test]
    fn strength_follows_the_calibrated_thresholds() {
        assert_eq!(Strength::from_score(0.05), Strength::Strong);
        assert_eq!(Strength::from_score(0.03), Strength::Some);
        assert_eq!(Strength::from_score(0.01), Strength::Weak);
    }

    #[test]
    fn a_missing_model_is_explained() {
        let searcher = Searcher::start(
            std::env::temp_dir().join("sharpr-no-such-store.sqlite"),
            Arc::new(AtomicU64::new(0)),
        );
        // Tests run with an isolated data folder that has no model.
        if super::super::files::locate().is_err() {
            let error = searcher.prepare("beach").err().unwrap();
            assert!(error.contains("EmbeddingGemma 2") || error.contains("llama-server"));
        }
    }
}
