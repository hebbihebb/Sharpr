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
    /// Every indexed candidate, closest first.
    pub paths: Vec<PathBuf>,
    /// Candidates with a vector; the rest aren't indexed yet.
    pub indexed: usize,
    pub eligible: usize,
    /// None when too few images are indexed to judge (`JUDGE_FROM`).
    pub strength: Option<Strength>,
}

/// Ranks `candidates`, each with its current fingerprint if known.
pub fn rank(candidates: &[(PathBuf, Option<String>)], prepared: &Prepared) -> Ranked {
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
    /// `reindex` runs on the search thread when the vector file turns out to
    /// be deleted or replaced, so indexing can start again.
    pub fn start(
        store_path: PathBuf,
        vectors_version: Arc<AtomicU64>,
        reindex: Box<dyn Fn() + Send>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        let _ = std::thread::Builder::new()
            .name("visual-search".into())
            .spawn(move || {
                let mut state = State::new(store_path, vectors_version, reindex);
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

/// Size and modification time of each model file, which a cached model key
/// must still match.
type ModelStamps = [(u64, i64); 2];

/// Which file the vectors came from: device and inode, so a deleted and
/// recreated vector file is noticed.
type FileId = (u64, u64);

fn file_id(path: &std::path::Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

struct State {
    store_path: PathBuf,
    vectors_version: Arc<AtomicU64>,
    reindex: Box<dyn Fn() + Send>,
    server: Option<Server>,
    generic: Option<Vec<f32>>,
    /// The model key, and the files and stamps it was computed for.
    model: Option<(super::files::ModelFiles, ModelStamps, String)>,
    /// Vectors for (model key, version, vector file).
    loaded: Option<(String, u64, FileId, Vectors)>,
    /// The last query, which "Show More" asks for again.
    last: Option<(String, Vec<f32>)>,
}

impl State {
    fn new(
        store_path: PathBuf,
        vectors_version: Arc<AtomicU64>,
        reindex: Box<dyn Fn() + Send>,
    ) -> Self {
        Self {
            store_path,
            vectors_version,
            reindex,
            server: None,
            generic: None,
            model: None,
            loaded: None,
            last: None,
        }
    }

    /// The key of the installed model files, recomputed whenever a file's
    /// size or modification time changes. A new key drops the query server
    /// and everything computed with the old model.
    fn model_key(&mut self, files: &super::files::ModelFiles) -> Result<String, String> {
        let stamp = |path: &std::path::Path| {
            std::fs::metadata(path)
                .map(|m| crate::fingerprint::file_stamp(&m))
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        let stamps = [stamp(&files.model)?, stamp(&files.projector)?];
        if let Some((known, known_stamps, key)) = &self.model {
            if known == files && *known_stamps == stamps {
                return Ok(key.clone());
            }
        }
        let key = Store::open(&self.store_path)?.model_key(files)?;
        if self.model.as_ref().is_some_and(|(_, _, old)| *old != key) {
            self.server = None;
            self.generic = None;
            self.loaded = None;
        }
        self.last = None;
        self.model = Some((files.clone(), stamps, key.clone()));
        Ok(key)
    }

    /// The vectors for `model`, reloaded when indexing added more or the
    /// vector file was replaced. A vanished or replaced file asks for a new
    /// indexing pass.
    fn vectors(&mut self, model: &str) -> Result<Vectors, String> {
        let version = self.vectors_version.load(Ordering::Relaxed);
        let current = file_id(&self.store_path);
        if let Some((key, loaded_version, id, vectors)) = &self.loaded {
            if current == Some(*id) {
                if key == model && *loaded_version == version {
                    return Ok(vectors.clone());
                }
            } else {
                self.loaded = None;
                (self.reindex)();
            }
        }
        let vectors = Arc::new(Store::open(&self.store_path)?.vectors(model)?);
        let id = file_id(&self.store_path).ok_or("The visual search index could not be opened")?;
        self.loaded = Some((model.to_string(), version, id, vectors.clone()));
        Ok(vectors)
    }

    fn prepare(&mut self, text: &str) -> Result<Prepared, String> {
        let files = super::files::locate()?;
        let model = self.model_key(&files)?;
        let vectors = self.vectors(&model)?;
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
        let ranked = rank(&candidates, &prepared);
        assert_eq!(
            ranked.paths,
            vec![
                PathBuf::from("/a/copy-of-near.jpg"),
                PathBuf::from("/a/near.jpg"),
                PathBuf::from("/a/mid.jpg"),
                PathBuf::from("/a/far.jpg"),
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
        assert_eq!(rank(&many, &prepared).strength, Some(Strength::Strong));

        let none = rank(&candidates[4..], &prepared);
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

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-search-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_model_replaced_in_place_changes_the_key_and_drops_old_vectors() {
        let dir = temp_dir("model");
        let files = super::super::files::ModelFiles {
            model: dir.join("model.gguf"),
            projector: dir.join("projector.gguf"),
            server: dir.join("llama-server"),
            gpu_server: None,
        };
        std::fs::write(&files.model, b"GGUF first").unwrap();
        std::fs::write(&files.projector, b"GGUF projector").unwrap();
        let store_path = dir.join("visual-search.sqlite");
        let mut state = State::new(
            store_path.clone(),
            Arc::new(AtomicU64::new(0)),
            Box::new(|| {}),
        );
        let first = state.model_key(&files).unwrap();
        Store::open(&store_path)
            .unwrap()
            .put_vector(&first, "aa", &axis(0, 1.0))
            .unwrap();
        assert_eq!(state.vectors(&first).unwrap().len(), 1);
        assert_eq!(
            state.model_key(&files).unwrap(),
            first,
            "unchanged files reuse the key"
        );

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&files.model, b"GGUF second").unwrap();
        let second = state.model_key(&files).unwrap();
        assert_ne!(second, first);
        assert!(state.loaded.is_none(), "old-model vectors are dropped");
        assert!(
            state.vectors(&second).unwrap().is_empty(),
            "never ranks old vectors"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_deleted_vector_file_is_noticed_and_asks_for_reindexing() {
        let dir = temp_dir("store");
        let store_path = dir.join("visual-search.sqlite");
        Store::open(&store_path)
            .unwrap()
            .put_vector("m", "aa", &axis(0, 1.0))
            .unwrap();
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = asked.clone();
        let mut state = State::new(
            store_path.clone(),
            Arc::new(AtomicU64::new(0)),
            Box::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        );
        assert_eq!(state.vectors("m").unwrap().len(), 1);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", store_path.display()));
        }
        assert!(
            state.vectors("m").unwrap().is_empty(),
            "no cached vectors survive"
        );
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        assert!(state.vectors("m").unwrap().is_empty());
        assert_eq!(
            asked.load(Ordering::Relaxed),
            1,
            "asked once, not on every search"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_model_is_explained() {
        let searcher = Searcher::start(
            std::env::temp_dir().join("sharpr-no-such-store.sqlite"),
            Arc::new(AtomicU64::new(0)),
            Box::new(|| {}),
        );
        // Tests run with an isolated data folder that has no model.
        if super::super::files::locate().is_err() {
            let error = searcher.prepare("beach").err().unwrap();
            assert!(error.contains("EmbeddingGemma 2") || error.contains("llama-server"));
        }
    }
}
