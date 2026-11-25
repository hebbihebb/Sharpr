//! The real-detail pass (D48): measures each fingerprinted image in the
//! enabled library once, on its own thread at nice 19 and idle I/O, after
//! each fingerprint pass. Each result is written as soon as it is known, so
//! a pass resumes where it stopped. Identical copies are measured once. A
//! full decode waits while visible thumbnails are queued, so the filmstrip
//! never waits on it. GTK-free.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::config::FolderMode;
use crate::detail::Stored;
use crate::library_index::LibraryIndex;

/// What a pass covers: the active library outside its disabled folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub roots: Vec<PathBuf>,
    pub disabled: Vec<PathBuf>,
}

/// How far measuring has got, for Library Overview.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    /// Images in the enabled library with a current fingerprint.
    pub images: usize,
    /// Of those, measured (including not measurable and unreadable).
    pub measured: usize,
    pub running: bool,
    /// Images measured per second in the current pass, once known.
    pub per_second: Option<f64>,
}

/// Measures every fingerprinted image under `roots` outside `disabled` that
/// this version of the method hasn't measured. `busy` holds a decode back
/// while visible thumbnails wait.
pub fn run_pass(
    index: &LibraryIndex,
    job: &Job,
    canceled: &dyn Fn() -> bool,
    busy: &dyn Fn() -> bool,
    progress: &dyn Fn(Progress),
    measure: &dyn Fn(&Path) -> Result<crate::detail::Detail, String>,
) -> Result<Progress, String> {
    let paths = crate::model::library::collect_library_image_paths_cancellable(
        &job.roots,
        FolderMode::DrillDown,
        None,
        &job.disabled,
        canceled,
    );
    if canceled() {
        return Err("Measuring canceled".into());
    }
    let cached = index.cached_fingerprints().map_err(|e| e.to_string())?;
    let done = index.details().map_err(|e| e.to_string())?;
    let mut state = Progress {
        running: true,
        ..Default::default()
    };
    // Fingerprints still to measure, each with one current file.
    let mut todo: Vec<(String, PathBuf, (u64, i64))> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
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
        state.images += 1;
        if done.contains_key(hash) {
            state.measured += 1;
        } else {
            *copies.entry(hash.clone()).or_default() += 1;
            if seen.insert(hash.clone()) {
                todo.push((hash.clone(), path.clone(), stamp));
            }
        }
    }
    progress(state);
    let started = std::time::Instant::now();
    let mut measured_now = 0usize;
    for (hash, path, stamp) in todo {
        // Visible thumbnails first; a quiet moment is enough to go on.
        while busy() {
            if canceled() {
                return Err("Measuring canceled".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if canceled() {
            return Err("Measuring canceled".into());
        }
        let unchanged =
            || std::fs::metadata(&path).is_ok_and(|m| crate::fingerprint::file_stamp(&m) == stamp);
        if !unchanged() {
            continue;
        }
        let stored = match measure(&path) {
            Ok(detail) => Stored::from_detail(&detail),
            Err(error) => {
                crate::bench_warn!(
                    "detail.image_failed",
                    serde_json::json!({"path": path.display().to_string(), "error": error}),
                );
                Stored::failed()
            }
        };
        // An edit while measuring must not lend its content to the old
        // fingerprint; the next fingerprint pass brings the file back.
        if canceled() {
            return Err("Measuring canceled".into());
        }
        if !unchanged() {
            continue;
        }
        index
            .store_detail(&hash, &stored)
            .map_err(|e| e.to_string())?;
        state.measured += copies.get(&hash).copied().unwrap_or(1);
        measured_now += 1;
        let seconds = started.elapsed().as_secs_f64();
        if seconds >= 5.0 {
            state.per_second = Some(measured_now as f64 / seconds);
        }
        progress(state);
    }
    state.running = false;
    Ok(state)
}

#[derive(Default)]
struct Queue {
    pending: Option<Job>,
    running: Option<Job>,
}

type Busy = Arc<dyn Fn() -> bool + Send + Sync>;

/// The measuring thread. The newest request wins; a request for a different
/// library or set of disabled folders cancels the running pass.
pub struct Service {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    busy: Arc<Mutex<Option<Busy>>>,
}

impl Service {
    pub fn start(index: Arc<LibraryIndex>) -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress::default()));
        let busy: Arc<Mutex<Option<Busy>>> = Arc::new(Mutex::new(None));
        let (thread_queue, thread_cancel, thread_progress, thread_busy) = (
            queue.clone(),
            cancel.clone(),
            progress.clone(),
            busy.clone(),
        );
        let _ = std::thread::Builder::new()
            .name("detail".into())
            .spawn(move || {
                #[cfg(not(test))]
                crate::fingerprint::lower_thread_priority();
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
                    let canceled = || thread_cancel.load(Ordering::Relaxed);
                    let busy_now = || {
                        thread_busy
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .as_ref()
                            .is_some_and(|b| b())
                    };
                    let report = |p: Progress| {
                        let mut slot = thread_progress.lock().unwrap_or_else(|e| e.into_inner());
                        *slot = p;
                    };
                    let result = run_pass(
                        &index,
                        &job,
                        &canceled,
                        &busy_now,
                        &report,
                        &crate::detail::measure_path,
                    );
                    {
                        let mut slot = thread_progress.lock().unwrap_or_else(|e| e.into_inner());
                        slot.running = false;
                        if let Ok(done) = result {
                            *slot = done;
                        }
                    }
                    let (lock, _) = &*thread_queue;
                    lock.lock().unwrap_or_else(|e| e.into_inner()).running = None;
                }
            });
        Self {
            queue,
            cancel,
            progress,
            busy,
        }
    }

    /// While `busy` says so, decodes wait (visible thumbnails queued).
    pub fn set_busy(&self, busy: impl Fn() -> bool + Send + Sync + 'static) {
        *self.busy.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(busy));
    }

    /// Asks for a pass over `job`'s scope; a different scope stops the
    /// running pass.
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

    pub fn progress(&self) -> Progress {
        *self.progress.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detail::{Detail, Estimate};
    use std::cell::Cell;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-detail-index-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake(width: u32, height: u32, estimate: Estimate) -> Detail {
        Detail {
            width,
            height,
            estimate,
            textured_tiles: 100,
            losses: Vec::new(),
            focus: None,
        }
    }

    #[test]
    fn a_pass_measures_each_fingerprint_once_and_skips_disabled_folders() {
        let root = temp("pass");
        let disabled = root.join("off");
        std::fs::create_dir_all(&disabled).unwrap();
        let index = LibraryIndex::open_in_memory().unwrap();
        let mut rows = Vec::new();
        for (name, hash) in [
            ("a.png", "A"),
            ("copy-of-a.png", "A"),
            ("b.png", "B"),
            ("off/c.png", "C"),
        ] {
            let path = root.join(name);
            image::RgbImage::new(4, 4).save(&path).unwrap();
            let (size, mtime) = crate::fingerprint::file_stamp(&std::fs::metadata(&path).unwrap());
            rows.push((path, size, mtime, hash.to_string()));
        }
        index.store_fingerprints(&rows).unwrap();
        let job = Job {
            roots: vec![root.clone()],
            disabled: vec![disabled.clone()],
        };
        let calls = Cell::new(0);
        let measure = |path: &Path| {
            calls.set(calls.get() + 1);
            assert!(!path.starts_with(&disabled), "disabled folders are skipped");
            Ok(if path.ends_with("b.png") {
                fake(3840, 2160, Estimate::Below { short_side: 1080 })
            } else {
                fake(3840, 2160, Estimate::Full)
            })
        };
        let done = run_pass(&index, &job, &|| false, &|| false, &|_| {}, &measure).unwrap();
        assert_eq!(calls.get(), 2, "identical copies are measured once");
        assert_eq!((done.images, done.measured), (3, 3));
        let details = index.details().unwrap();
        assert!(details["B"].looks_upscaled());
        assert_eq!(details["B"].detail, Some((1920, 1080)));
        assert_eq!(details["A"].words(), "Real detail: full 3840×2160");
        // A second pass has nothing left to do.
        let again = run_pass(&index, &job, &|| false, &|| false, &|_| {}, &measure).unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(again.measured, 3);
    }

    #[test]
    fn decodes_wait_while_visible_thumbnails_are_queued() {
        let root = temp("busy");
        let index = LibraryIndex::open_in_memory().unwrap();
        let path = root.join("a.png");
        image::RgbImage::new(4, 4).save(&path).unwrap();
        let (size, mtime) = crate::fingerprint::file_stamp(&std::fs::metadata(&path).unwrap());
        index
            .store_fingerprints(&[(path.clone(), size, mtime, "A".into())])
            .unwrap();
        let job = Job {
            roots: vec![root],
            disabled: Vec::new(),
        };
        let checks = Cell::new(0);
        let busy = || {
            checks.set(checks.get() + 1);
            checks.get() < 4
        };
        let measured = Cell::new(false);
        let measure = |_: &Path| {
            assert!(checks.get() >= 4, "waited for the thumbnails first");
            measured.set(true);
            Ok(fake(100, 100, Estimate::Full))
        };
        run_pass(&index, &job, &|| false, &busy, &|_| {}, &measure).unwrap();
        assert!(measured.get());
    }

    #[test]
    fn an_unreadable_image_is_recorded_and_not_retried() {
        let root = temp("failed");
        let index = LibraryIndex::open_in_memory().unwrap();
        let path = root.join("broken.png");
        std::fs::write(&path, b"not an image").unwrap();
        let (size, mtime) = crate::fingerprint::file_stamp(&std::fs::metadata(&path).unwrap());
        index
            .store_fingerprints(&[(path, size, mtime, "X".into())])
            .unwrap();
        let job = Job {
            roots: vec![root],
            disabled: Vec::new(),
        };
        run_pass(
            &index,
            &job,
            &|| false,
            &|| false,
            &|_| {},
            &crate::detail::measure_path,
        )
        .unwrap();
        assert_eq!(
            index.details().unwrap()["X"].status,
            crate::detail::Status::Failed
        );
    }

    #[test]
    fn changing_scope_cancels_the_running_pass_and_skips_the_disabled_folder() {
        let root = temp("scope-cancel");
        let off = root.join("off");
        std::fs::create_dir_all(&off).unwrap();
        let index = Arc::new(LibraryIndex::open_in_memory().unwrap());
        let mut rows = Vec::new();
        for (name, hash) in [("keep.png", "K"), ("off/skip.png", "S")] {
            let path = root.join(name);
            image::RgbImage::new(4, 4).save(&path).unwrap();
            let (size, mtime) = crate::fingerprint::file_stamp(&std::fs::metadata(&path).unwrap());
            rows.push((path, size, mtime, hash.to_string()));
        }
        index.store_fingerprints(&rows).unwrap();
        let service = Service::start(index.clone());
        let paused = Arc::new(AtomicBool::new(true));
        let paused_probe = paused.clone();
        service.set_busy(move || paused_probe.load(Ordering::Relaxed));
        service.request(Job {
            roots: vec![root.clone()],
            disabled: Vec::new(),
        });
        let started = std::time::Instant::now();
        while !service.progress().running && started.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(service.progress().running);
        // Now disable `off`: the old pass cancels and the replacement skips it.
        service.request(Job {
            roots: vec![root.clone()],
            disabled: vec![off.clone()],
        });
        paused.store(false, Ordering::Relaxed);
        let started = std::time::Instant::now();
        while (service.progress().running || service.progress().images != 1)
            && started.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let progress = service.progress();
        assert_eq!((progress.images, progress.measured), (1, 1));
        let details = index.details().unwrap();
        assert!(details.contains_key("K"));
        assert!(
            !details.contains_key("S"),
            "disabled folder was not measured"
        );
    }
}
