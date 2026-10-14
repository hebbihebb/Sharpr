//! Indexing: a vector for every image in the enabled library.
//!
//! A pass runs after each fingerprint pass, on its own thread at nice 19 and
//! idle I/O. It embeds each fingerprint that has no vector yet (identical
//! copies once), writes each vector as soon as it has it, and so resumes
//! where it stopped after a quit or a cancel. An image that can't be read is
//! recorded and not retried until the model changes. The `llama-server`
//! child runs only while a pass has work and stops when it ends.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use super::files::ModelFiles;
use super::runtime::{Mode, Server};
use super::store::Store;
use crate::config::FolderMode;
use crate::library_index::LibraryIndex;

/// How much of the enabled library has vectors, counted in images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub images: usize,
    pub indexed: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// No pass has started yet.
    Waiting,
    /// The model or runtime isn't installed or doesn't work; the message
    /// says what to do.
    Unavailable(String),
    Indexing(Coverage),
    Done(Coverage),
    Failed(String),
}

/// Why one image has no vector.
#[derive(Debug)]
pub enum EmbedError {
    /// This image can't be used; record it and go on.
    Image(String),
    /// The runtime is broken; stop the pass and try again next time.
    Runtime(String),
}

pub trait Embedder {
    fn embed_image(&mut self, path: &Path) -> Result<Vec<f32>, EmbedError>;
}

/// Embeds through a `Mode::Images` server, started at the first image.
pub struct ServerEmbedder {
    files: ModelFiles,
    use_gpu: bool,
    server: Option<Server>,
    /// The graphics card the running server uses, if any.
    on_gpu: Option<String>,
    /// The GPU didn't work during this pass; the processor takes over.
    gpu_failed: bool,
    crashes: usize,
    /// Where the running server works, for the status line.
    device: Arc<Mutex<Option<String>>>,
}

impl ServerEmbedder {
    pub fn new(files: ModelFiles, use_gpu: bool, device: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            files,
            use_gpu,
            server: None,
            on_gpu: None,
            gpu_failed: false,
            crashes: 0,
            device,
        }
    }

    /// Starts a server: on a graphics card with room when the GPU build is
    /// installed and allowed, otherwise on the processor.
    fn ensure_server(&mut self) -> Result<(), EmbedError> {
        if self.server.is_some() {
            return Ok(());
        }
        self.on_gpu = None;
        let gpu = (self.use_gpu && !self.gpu_failed)
            .then(|| self.files.gpu_server.clone())
            .flatten();
        if let Some(gpu_server) = gpu {
            match super::runtime::pick_gpu(&gpu_server) {
                Some(device) => match Server::start_gpu(&self.files, &device) {
                    Ok(server) => {
                        self.server = Some(server);
                        self.on_gpu = Some(device.name);
                    }
                    Err(error) => {
                        crate::bench_warn!(
                            "visual_search.gpu.fail",
                            serde_json::json!({"device": device.name, "error": error}),
                        );
                        self.gpu_failed = true;
                    }
                },
                None => self.gpu_failed = true,
            }
        }
        if self.server.is_none() {
            self.server =
                Some(Server::start(&self.files, Mode::Images).map_err(EmbedError::Runtime)?);
        }
        crate::bench_event!(
            "visual_search.server",
            serde_json::json!({"device": self.on_gpu.as_deref().unwrap_or("processor")}),
        );
        *self.device.lock().unwrap_or_else(|e| e.into_inner()) = self.on_gpu.clone();
        Ok(())
    }
}

impl Drop for ServerEmbedder {
    fn drop(&mut self) {
        *self.device.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl Embedder for ServerEmbedder {
    fn embed_image(&mut self, path: &Path) -> Result<Vec<f32>, EmbedError> {
        let png = super::prepare::prepare_png(path).map_err(EmbedError::Image)?;
        self.ensure_server()?;
        let server = self.server.as_mut().expect("started above");
        match server.embed_png(&png) {
            Ok(vector) => Ok(vector),
            Err(error) if server.is_alive() => {
                // The server rejected this image but still works.
                if error.contains("not EmbeddingGemma 2") {
                    Err(EmbedError::Runtime(error))
                } else {
                    Err(EmbedError::Image(error))
                }
            }
            Err(error) if self.on_gpu.is_some() => {
                // The graphics card failed, perhaps another program took its
                // memory: finish the pass on the processor, this image too.
                crate::bench_warn!(
                    "visual_search.gpu.fail",
                    serde_json::json!({"device": self.on_gpu, "error": error}),
                );
                self.server = None;
                self.gpu_failed = true;
                self.embed_image(path)
            }
            Err(error) => {
                // The image took the server down: skip it, restart for the
                // next one, and give up if that keeps happening.
                self.server = None;
                self.crashes += 1;
                if self.crashes >= 3 {
                    Err(EmbedError::Runtime(format!(
                        "llama-server keeps stopping: {error}"
                    )))
                } else {
                    Err(EmbedError::Image(format!(
                        "llama-server stopped while reading this image: {error}"
                    )))
                }
            }
        }
    }
}

/// Indexes every image under `root` outside `disabled` whose fingerprint has
/// no vector or failure for `model`. Images without a current fingerprint
/// wait for the next fingerprint pass. Returns the coverage reached.
#[allow(clippy::too_many_arguments)]
pub fn run_pass(
    index: &LibraryIndex,
    store: &Store,
    model: &str,
    root: &Path,
    disabled: &[PathBuf],
    canceled: &dyn Fn() -> bool,
    progress: &dyn Fn(Coverage),
    embedder: &mut dyn Embedder,
) -> Result<Coverage, String> {
    let paths = crate::model::library::collect_library_image_paths_cancellable(
        Some(root.to_path_buf()),
        FolderMode::DrillDown,
        None,
        disabled,
        canceled,
    );
    if canceled() {
        return Err("Indexing canceled".into());
    }
    let cached = index.cached_fingerprints().map_err(|e| e.to_string())?;
    let done = store.done(model)?;
    let failed = store.failures(model)?;
    let mut coverage = Coverage {
        images: paths.len(),
        ..Default::default()
    };
    // Fingerprints still to index, each with one of its files and how many
    // images it stands for.
    let mut todo: Vec<(String, PathBuf)> = Vec::new();
    let mut copies: HashMap<String, usize> = HashMap::new();
    for path in &paths {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        let stamp = crate::fingerprint::file_stamp(&metadata);
        let Some((size, mtime, hash)) = cached.get(path.to_string_lossy().as_ref()) else {
            continue;
        };
        if (*size, *mtime) != stamp {
            continue;
        }
        if failed.contains_key(hash) {
            coverage.failed += 1;
        } else if done.contains(hash) {
            coverage.indexed += 1;
        } else {
            let count = copies.entry(hash.clone()).or_insert(0);
            if *count == 0 {
                todo.push((hash.clone(), path.clone()));
            }
            *count += 1;
        }
    }
    progress(coverage);
    for (hash, path) in todo {
        if canceled() {
            return Err("Indexing canceled".into());
        }
        let images = copies[&hash];
        match embedder.embed_image(&path) {
            Ok(vector) => {
                store.put_vector(model, &hash, &vector)?;
                coverage.indexed += images;
            }
            Err(EmbedError::Image(error)) => {
                crate::bench_warn!(
                    "visual_search.image_failed",
                    serde_json::json!({"path": path.display().to_string(), "error": error}),
                );
                store.put_failure(model, &hash, &error)?;
                coverage.failed += images;
            }
            Err(EmbedError::Runtime(error)) => return Err(error),
        }
        progress(coverage);
    }
    Ok(coverage)
}

/// What a pass covers: the active library outside its disabled folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub root: PathBuf,
    pub disabled: Vec<PathBuf>,
}

#[derive(Default)]
struct Queue {
    pending: Option<Job>,
    running: Option<Job>,
}

/// The indexing thread. The newest request wins; a request for a different
/// library or set of disabled folders cancels the running pass.
pub struct Service {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    vectors_version: Arc<AtomicU64>,
    use_gpu: Arc<AtomicBool>,
    device: Arc<Mutex<Option<String>>>,
}

impl Service {
    /// `use_gpu` allows indexing on a graphics card (the GPU build must be
    /// installed); it can change later with `set_use_gpu`.
    pub fn start(index: Arc<LibraryIndex>, store_path: PathBuf, use_gpu: bool) -> Self {
        let use_gpu = Arc::new(AtomicBool::new(use_gpu));
        let device = Arc::new(Mutex::new(None));
        let (thread_use_gpu, thread_device) = (use_gpu.clone(), device.clone());
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status::Waiting));
        let vectors_version = Arc::new(AtomicU64::new(0));
        let (thread_queue, thread_cancel, thread_status, thread_version) = (
            queue.clone(),
            cancel.clone(),
            status.clone(),
            vectors_version.clone(),
        );
        let _ = std::thread::Builder::new()
            .name("visual-index".into())
            .spawn(move || {
                // The server inherits this thread's priority.
                #[cfg(not(test))]
                crate::fingerprint::lower_thread_priority();
                let set_status = |s: Status| {
                    *thread_status.lock().unwrap_or_else(|e| e.into_inner()) = s;
                };
                loop {
                    let job = {
                        let (lock, ready) = &*thread_queue;
                        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(job) = queue.pending.take() {
                                queue.running = Some(job.clone());
                                break job;
                            }
                            queue = ready.wait(queue).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    thread_cancel.store(false, Ordering::Relaxed);
                    let started = std::time::Instant::now();
                    let outcome = (|| -> Result<Option<Coverage>, String> {
                        let files = match super::files::locate() {
                            Ok(files) => files,
                            Err(message) => {
                                set_status(Status::Unavailable(message));
                                return Ok(None);
                            }
                        };
                        let store = Store::open(&store_path)?;
                        let model = store.model_key(&files)?;
                        let mut embedder = ServerEmbedder::new(
                            files,
                            thread_use_gpu.load(Ordering::Relaxed),
                            thread_device.clone(),
                        );
                        let canceled = || thread_cancel.load(Ordering::Relaxed);
                        let last_indexed = std::cell::Cell::new(None);
                        let progress = |coverage: Coverage| {
                            if last_indexed
                                .replace(Some(coverage.indexed))
                                .is_some_and(|last| last != coverage.indexed)
                            {
                                thread_version.fetch_add(1, Ordering::Relaxed);
                            }
                            set_status(Status::Indexing(coverage));
                        };
                        run_pass(
                            &index,
                            &store,
                            &model,
                            &job.root,
                            &job.disabled,
                            &canceled,
                            &progress,
                            &mut embedder,
                        )
                        .map(Some)
                    })();
                    match outcome {
                        Ok(Some(coverage)) => {
                            crate::bench_event!(
                                "visual_search.pass",
                                serde_json::json!({
                                    "images": coverage.images,
                                    "indexed": coverage.indexed,
                                    "failed": coverage.failed,
                                    "duration_ms": crate::bench::duration_ms(started),
                                }),
                            );
                            set_status(Status::Done(coverage));
                        }
                        Ok(None) => {}
                        Err(_) if thread_cancel.load(Ordering::Relaxed) => {}
                        Err(error) => {
                            crate::bench_warn!(
                                "visual_search.pass.fail",
                                serde_json::json!({"error": error}),
                            );
                            set_status(Status::Failed(error));
                        }
                    }
                    thread_version.fetch_add(1, Ordering::Relaxed);
                    let (lock, _) = &*thread_queue;
                    lock.lock().unwrap_or_else(|e| e.into_inner()).running = None;
                }
            });
        Self {
            queue,
            cancel,
            status,
            vectors_version,
            use_gpu,
            device,
        }
    }

    /// Applies from the next pass on.
    pub fn set_use_gpu(&self, enabled: bool) {
        self.use_gpu.store(enabled, Ordering::Relaxed);
    }

    /// The graphics card the running pass uses; None while idle or on the
    /// processor.
    pub fn device(&self) -> Option<String> {
        self.device
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Asks for a pass over `job`'s library.
    pub fn request(&self, job: Job) {
        let (lock, ready) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        if queue
            .running
            .as_ref()
            .is_some_and(|running| *running != job)
        {
            self.cancel.store(true, Ordering::Relaxed);
        }
        queue.pending = Some(job);
        ready.notify_one();
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Changes whenever vectors may have been added, so search reloads them.
    pub fn vectors_version(&self) -> Arc<AtomicU64> {
        self.vectors_version.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Embeds by file contents; "bad" files fail, "crash" breaks the runtime.
    struct FakeEmbedder {
        calls: Vec<PathBuf>,
    }

    impl Embedder for FakeEmbedder {
        fn embed_image(&mut self, path: &Path) -> Result<Vec<f32>, EmbedError> {
            self.calls.push(path.to_path_buf());
            let bytes = std::fs::read(path).unwrap();
            match bytes.as_slice() {
                b"bad" => Err(EmbedError::Image("unreadable".into())),
                b"crash" => Err(EmbedError::Runtime("runtime down".into())),
                _ => {
                    let mut v = vec![0.0; super::super::DIMENSIONS];
                    v[bytes.len() % super::super::DIMENSIONS] = 1.0;
                    Ok(v)
                }
            }
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-visual-index-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fingerprint(index: &LibraryIndex, root: &Path, disabled: &[PathBuf]) {
        crate::fingerprint::run_pass(index, root, disabled, &[], &|| false, &|_| {}).unwrap();
    }

    #[test]
    fn a_pass_indexes_each_fingerprint_once_and_resumes() {
        let root = temp_dir("pass");
        std::fs::create_dir_all(root.join("off")).unwrap();
        std::fs::write(root.join("a.jpg"), b"a").unwrap();
        std::fs::write(root.join("copy-of-a.jpg"), b"a").unwrap();
        std::fs::write(root.join("b.png"), b"bb").unwrap();
        std::fs::write(root.join("broken.jpg"), b"bad").unwrap();
        std::fs::write(root.join("off/c.jpg"), b"ccc").unwrap();
        let disabled = vec![root.join("off")];
        let index = LibraryIndex::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        fingerprint(&index, &root, &disabled);
        std::fs::write(root.join("new.jpg"), b"new").unwrap(); // no fingerprint yet

        let mut embedder = FakeEmbedder { calls: Vec::new() };
        let coverage = run_pass(
            &index,
            &store,
            "m",
            &root,
            &disabled,
            &|| false,
            &|_| {},
            &mut embedder,
        )
        .unwrap();
        assert_eq!(
            coverage,
            Coverage {
                images: 5,
                indexed: 3,
                failed: 1
            },
            "identical copies are embedded once and both count; the unfingerprinted image waits"
        );
        assert_eq!(embedder.calls.len(), 3);
        assert!(embedder
            .calls
            .iter()
            .all(|p| !p.starts_with(root.join("off"))));

        fingerprint(&index, &root, &disabled);
        let mut again = FakeEmbedder { calls: Vec::new() };
        let coverage = run_pass(
            &index,
            &store,
            "m",
            &root,
            &disabled,
            &|| false,
            &|_| {},
            &mut again,
        )
        .unwrap();
        assert_eq!(
            again.calls,
            vec![root.join("new.jpg")],
            "only new work, failures not retried"
        );
        assert_eq!((coverage.indexed, coverage.failed), (4, 1));

        let mut other_model = FakeEmbedder { calls: Vec::new() };
        run_pass(
            &index,
            &store,
            "m2",
            &root,
            &disabled,
            &|| false,
            &|_| {},
            &mut other_model,
        )
        .unwrap();
        assert_eq!(
            other_model.calls.len(),
            4,
            "a new model indexes every distinct image again"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_broken_runtime_stops_the_pass_and_keeps_finished_vectors() {
        let root = temp_dir("runtime");
        std::fs::write(root.join("a.jpg"), b"a").unwrap();
        std::fs::write(root.join("z.jpg"), b"crash").unwrap();
        let index = LibraryIndex::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        fingerprint(&index, &root, &[]);
        let mut embedder = FakeEmbedder { calls: Vec::new() };
        let error = run_pass(
            &index,
            &store,
            "m",
            &root,
            &[],
            &|| false,
            &|_| {},
            &mut embedder,
        )
        .unwrap_err();
        assert_eq!(error, "runtime down");
        assert_eq!(store.done("m").unwrap().len(), embedder.calls.len() - 1);
        assert!(
            store.failures("m").unwrap().is_empty(),
            "not the image's fault"
        );

        let canceled = run_pass(
            &index,
            &store,
            "m",
            &root,
            &[],
            &|| true,
            &|_| {},
            &mut embedder,
        );
        assert!(canceled.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Runs the app's own preparation and runtime over the Phase 3 sample
    /// and writes a report in the benchmark's format, for
    /// `tools/benchmark-parity.py` and `tools/benchmark-score.py`. Reads
    /// private images; development only.
    ///
    /// `SHARPR_TEST_MODEL_DIR`, `SHARPR_LLAMA_SERVER`, `SHARPR_BENCH_MANIFEST`
    /// and `SHARPR_BENCH_OUTPUT` (a `.json` path) are required;
    /// `SHARPR_BENCH_LIMIT` takes the first N images.
    #[test]
    #[ignore]
    fn app_pipeline_benchmark_report() {
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let files = super::super::files::locate_in(
            Path::new(&env("SHARPR_TEST_MODEL_DIR")),
            &[PathBuf::from(env("SHARPR_LLAMA_SERVER"))],
        )
        .unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(env("SHARPR_BENCH_MANIFEST")).unwrap()).unwrap();
        let mut images: Vec<PathBuf> = manifest["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| PathBuf::from(p.as_str().unwrap()))
            .collect();
        if let Ok(limit) = std::env::var("SHARPR_BENCH_LIMIT") {
            images.truncate(limit.parse().unwrap());
        }
        let output = PathBuf::from(env("SHARPR_BENCH_OUTPUT"));

        let use_gpu = files.gpu_server.is_some();
        let mut embedder = ServerEmbedder::new(files.clone(), use_gpu, Arc::new(Mutex::new(None)));
        let (mut vectors, mut embedded, mut errors) = (Vec::new(), Vec::new(), Vec::new());
        let started = std::time::Instant::now();
        for (i, path) in images.iter().enumerate() {
            match embedder.embed_image(path) {
                Ok(vector) => {
                    vectors.push(vector);
                    embedded.push(path.clone());
                }
                Err(error) => errors.push(serde_json::json!({
                    "path": path.display().to_string(),
                    "error": format!("{error:?}"),
                })),
            }
            if i % 50 == 0 {
                eprintln!(
                    "{} of {} in {:.0} s",
                    i + 1,
                    images.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
        let index_seconds = started.elapsed().as_secs_f64();
        drop(embedder);

        let mut queries = Server::start(&files, Mode::Queries).unwrap();
        let mut results = Vec::new();
        let mut latencies = Vec::new();
        for query in manifest["queries"].as_array().unwrap() {
            let text = query["text"].as_str().unwrap();
            let began = std::time::Instant::now();
            let q = queries.embed_query(text).unwrap();
            let mut ranked: Vec<(f32, usize)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (super::super::similarity(&q, v), i))
                .collect();
            ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
            ranked.truncate(20);
            latencies.push(began.elapsed().as_secs_f64());
            results.push(serde_json::json!({
                "text": text,
                "top20": ranked.iter().map(|(_, i)| embedded[*i].display().to_string()).collect::<Vec<_>>(),
                "scores": ranked.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
                "latency_seconds": latencies.last(),
            }));
        }
        let mut sorted = latencies.clone();
        sorted.sort_by(f64::total_cmp);
        let report = serde_json::json!({
            "images": embedded.len(),
            "index_seconds": index_seconds,
            "images_per_second": embedded.len() as f64 / index_seconds,
            "warm_p95_seconds": sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)],
            "provenance": {"pipeline": "Sharpr visual_search", "preparation_version": super::super::PREPARATION_VERSION},
            "errors": errors,
            "results": results,
        });
        std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

        // The vectors as a NumPy .npy file beside the report.
        let header = format!(
            "{{'descr': '<f4', 'fortran_order': False, 'shape': ({}, {}), }}",
            vectors.len(),
            super::super::DIMENSIONS
        );
        let padded = (10 + header.len() + 1).div_ceil(64) * 64 - 10;
        let mut npy = b"\x93NUMPY\x01\x00".to_vec();
        npy.extend_from_slice(&(padded as u16).to_le_bytes());
        npy.extend_from_slice(header.as_bytes());
        npy.resize(10 + padded - 1, b' ');
        npy.push(b'\n');
        for vector in &vectors {
            for value in vector {
                npy.extend_from_slice(&value.to_le_bytes());
            }
        }
        std::fs::write(output.with_extension("vectors.npy"), npy).unwrap();
    }

    /// The service end to end with the installed model: run with
    /// `XDG_DATA_HOME` pointing at a folder whose
    /// `sharpr-models/embeddinggemma-2/` holds the model and `runtime/`.
    #[test]
    #[ignore]
    fn service_indexes_a_library_with_the_installed_model() {
        let root = temp_dir("service");
        for (name, colour) in [("red.png", [200, 30, 30]), ("blue.png", [30, 30, 200])] {
            image::RgbImage::from_pixel(64, 48, image::Rgb(colour))
                .save(root.join(name))
                .unwrap();
        }
        std::fs::write(root.join("broken.jpg"), b"not an image").unwrap();
        let index = Arc::new(LibraryIndex::open_in_memory().unwrap());
        fingerprint(&index, &root, &[]);
        let store_path = root.join("store/visual-search.sqlite");
        let service = Service::start(index, store_path.clone(), true);
        service.request(Job {
            root: root.clone(),
            disabled: Vec::new(),
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        let coverage = loop {
            match service.status() {
                Status::Done(coverage) => break coverage,
                Status::Unavailable(message) | Status::Failed(message) => panic!("{message}"),
                _ => {}
            }
            assert!(std::time::Instant::now() < deadline, "indexing timed out");
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        assert_eq!(
            coverage,
            Coverage {
                images: 3,
                indexed: 2,
                failed: 1
            }
        );
        assert!(service.vectors_version().load(Ordering::Relaxed) > 0);
        let store = Store::open(&store_path).unwrap();
        let files = super::super::files::locate().unwrap();
        let vectors = store.vectors(&store.model_key(&files).unwrap()).unwrap();
        assert_eq!(vectors.len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
}
