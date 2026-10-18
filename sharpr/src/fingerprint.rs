//! Content fingerprints: an image's identity independent of its path
//! (`ROADMAP.md` Phase 4).
//!
//! A fingerprint is the xxh3-128 hash of a file's bytes. Results are cached
//! in the library index by path, size, and modification time, so a pass
//! hashes only new or changed files. Passes run on a background thread at the
//! lowest CPU and I/O priority and never read disabled folders.

use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::config::FolderMode;
use crate::library_index::LibraryIndex;

/// The xxh3-128 hash of the file's bytes as 32 lowercase hex digits. Pages
/// read for hashing are dropped from the page cache afterwards, so a pass
/// over the library doesn't push out data the viewer and filmstrip need.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = twox_hash::XxHash3_128::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
    }
    // SAFETY: the descriptor is open for the duration of the call.
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
    Ok(format!("{:032x}", hasher.finish_128()))
}

/// Size and modification time (nanoseconds) that a cached fingerprint must
/// match to be reused.
pub fn file_stamp(metadata: &std::fs::Metadata) -> (u64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        metadata.len(),
        metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec(),
    )
}

/// Lowers the calling thread to nice 19 and the idle I/O class, so hashing
/// only uses time and disk bandwidth nothing else wants.
#[cfg_attr(test, allow(dead_code))]
pub fn lower_thread_priority() {
    // SAFETY: plain syscalls on the calling thread's own id.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 19);
        const IOPRIO_WHO_PROCESS: libc::c_long = 1;
        const IOPRIO_CLASS_IDLE: libc::c_long = 3;
        const IOPRIO_CLASS_SHIFT: libc::c_long = 13;
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            tid as libc::c_long,
            IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT,
        );
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PassSummary {
    pub files: usize,
    pub hashed: usize,
    pub failed: usize,
}

/// Fingerprints every image under the `roots` outside `disabled` folders that has
/// no fresh cached fingerprint. `first` paths (such as bucket members) are
/// done before the rest. Stops early when `canceled` returns true; work done
/// so far is kept.
pub fn run_pass(
    index: &LibraryIndex,
    roots: &[PathBuf],
    disabled: &[PathBuf],
    first: &[PathBuf],
    canceled: &dyn Fn() -> bool,
    progress: &dyn Fn(Progress),
) -> Result<PassSummary, String> {
    let mut paths = crate::model::library::collect_library_image_paths_cancellable(
        roots,
        FolderMode::DrillDown,
        None,
        disabled,
        canceled,
    );
    if canceled() {
        return Ok(PassSummary::default());
    }
    let first: std::collections::HashSet<&PathBuf> = first.iter().collect();
    paths.sort_by_key(|p| !first.contains(p));
    let cached = index.cached_fingerprints().map_err(|e| e.to_string())?;
    let mut summary = PassSummary {
        files: paths.len(),
        ..Default::default()
    };
    let mut done = 0;
    let mut batch = Vec::new();
    for path in &paths {
        if canceled() {
            break;
        }
        let Ok(metadata) = std::fs::metadata(path) else {
            done += 1;
            continue;
        };
        let (size, mtime) = file_stamp(&metadata);
        let key = path.to_string_lossy();
        let fresh = cached
            .get(key.as_ref())
            .is_some_and(|(s, m, _)| *s == size && *m == mtime);
        if !fresh {
            match hash_file(path) {
                Ok(hash) => {
                    batch.push((path.clone(), size, mtime, hash));
                    summary.hashed += 1;
                }
                Err(_) => summary.failed += 1,
            }
        }
        done += 1;
        if batch.len() >= 64 {
            index
                .store_fingerprints(&std::mem::take(&mut batch))
                .map_err(|e| e.to_string())?;
        }
        if done % 50 == 0 {
            progress(Progress {
                done,
                total: paths.len(),
            });
        }
    }
    index
        .store_fingerprints(&batch)
        .map_err(|e| e.to_string())?;
    progress(Progress {
        done,
        total: paths.len(),
    });
    Ok(summary)
}

/// What a pass covers: the active library outside its disabled folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub roots: Vec<PathBuf>,
    pub disabled: Vec<PathBuf>,
}

impl Job {
    fn same_scope(&self, other: &Job) -> bool {
        self.roots == other.roots && self.disabled == other.disabled
    }
}

#[derive(Default)]
struct Queue {
    pending: Option<Job>,
    /// The pending pass need not wait for the spacing between passes.
    soon: bool,
    running: Option<Job>,
}

/// A background thread that runs fingerprint passes on request. The newest
/// request wins. A request for the same library while a pass runs queues
/// one more pass; a different library or set of disabled folders cancels it.
/// Passes over the same library start at least `MIN_INTERVAL` apart, unless
/// asked for with `request_soon`.
pub struct Service {
    queue: std::sync::Arc<(std::sync::Mutex<Queue>, std::sync::Condvar)>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress: std::sync::Arc<std::sync::Mutex<Option<Progress>>>,
}

/// Runs on the service thread after every completed pass.
pub type PassDone = Box<dyn Fn(&Job, &PassSummary) + Send>;

#[cfg(not(test))]
const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Tests drive passes directly and shouldn't wait.
#[cfg(test)]
const MIN_INTERVAL: std::time::Duration = std::time::Duration::ZERO;

impl Service {
    /// Starts the thread. `pass_done` runs on it after every completed pass.
    pub fn start(index: std::sync::Arc<LibraryIndex>, pass_done: PassDone) -> Self {
        Self::start_spaced(index, pass_done, MIN_INTERVAL)
    }

    fn start_spaced(
        index: std::sync::Arc<LibraryIndex>,
        pass_done: PassDone,
        interval: std::time::Duration,
    ) -> Self {
        let queue = std::sync::Arc::new((
            std::sync::Mutex::new(Queue::default()),
            std::sync::Condvar::new(),
        ));
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let progress = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (thread_queue, thread_cancel, thread_progress) =
            (queue.clone(), cancel.clone(), progress.clone());
        let _ = std::thread::Builder::new()
            .name("fingerprints".into())
            .spawn(move || {
                // Tests run under a busy machine and must finish in time.
                #[cfg(not(test))]
                lower_thread_priority();
                let mut last: Option<(Job, std::time::Instant)> = None;
                loop {
                    // The pending job stays queued while it waits out the
                    // spacing, so a newer request replaces it and a soon one
                    // ends the wait.
                    let job = {
                        let (lock, ready) = &*thread_queue;
                        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            let Some(job) = queue.pending.as_ref() else {
                                queue = ready.wait(queue).unwrap_or_else(|e| e.into_inner());
                                continue;
                            };
                            let wait = match &last {
                                Some((previous, started))
                                    if !queue.soon && previous.same_scope(job) =>
                                {
                                    interval.saturating_sub(started.elapsed())
                                }
                                _ => std::time::Duration::ZERO,
                            };
                            if !wait.is_zero() {
                                queue = ready
                                    .wait_timeout(queue, wait)
                                    .unwrap_or_else(|e| e.into_inner())
                                    .0;
                                continue;
                            }
                            let job = queue.pending.take().expect("checked above");
                            queue.soon = false;
                            queue.running = Some(job.clone());
                            break job;
                        }
                    };
                    thread_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                    let started = std::time::Instant::now();
                    let canceled = || thread_cancel.load(std::sync::atomic::Ordering::Relaxed);
                    let report = |p: Progress| {
                        *thread_progress.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
                    };
                    // Bucket members first, so re-linking can start soonest.
                    let first = crate::buckets::member_paths(&index);
                    match run_pass(
                        &index,
                        &job.roots,
                        &job.disabled,
                        &first,
                        &canceled,
                        &report,
                    ) {
                        Ok(summary) if !canceled() => {
                            crate::bench_event!(
                                "fingerprint.pass",
                                serde_json::json!({
                                    "files": summary.files,
                                    "hashed": summary.hashed,
                                    "failed": summary.failed,
                                    "duration_ms": crate::bench::duration_ms(started),
                                }),
                            );
                            pass_done(&job, &summary);
                        }
                        Ok(_) => {}
                        Err(error) => crate::bench_warn!(
                            "fingerprint.pass.fail",
                            serde_json::json!({"error": error}),
                        ),
                    }
                    last = Some((job, started));
                    let (lock, _) = &*thread_queue;
                    lock.lock().unwrap_or_else(|e| e.into_inner()).running = None;
                }
            });
        Self {
            queue,
            cancel,
            progress,
        }
    }

    /// Asks for a pass over `job`'s library.
    pub fn request(&self, job: Job) {
        self.enqueue(job, false);
    }

    /// Like `request`, but the pass starts without waiting for the spacing
    /// between passes: something under the roots changed.
    pub fn request_soon(&self, job: Job) {
        self.enqueue(job, true);
    }

    fn enqueue(&self, job: Job, soon: bool) {
        let (lock, ready) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        if queue
            .running
            .as_ref()
            .is_some_and(|running| !running.same_scope(&job))
        {
            self.cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        queue.pending = Some(job);
        queue.soon |= soon;
        ready.notify_one();
    }

    /// Progress of the latest pass, if one has started.
    pub fn progress(&self) -> Option<Progress> {
        *self.progress.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-fingerprint-{}-{}-{name}",
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
    fn identical_bytes_share_a_fingerprint_and_any_change_alters_it() {
        let dir = temp_dir("hash");
        std::fs::write(dir.join("a.jpg"), b"same bytes").unwrap();
        std::fs::write(dir.join("copy.jpg"), b"same bytes").unwrap();
        std::fs::write(dir.join("b.jpg"), b"same byteS").unwrap();
        let a = hash_file(&dir.join("a.jpg")).unwrap();
        assert_eq!(a.len(), 32);
        assert_eq!(a, hash_file(&dir.join("copy.jpg")).unwrap());
        assert_ne!(a, hash_file(&dir.join("b.jpg")).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_soon_request_skips_the_spacing_between_passes() {
        let root = temp_dir("soon");
        std::fs::write(root.join("a.jpg"), b"a").unwrap();
        let index = std::sync::Arc::new(LibraryIndex::open_in_memory().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let service = Service::start_spaced(
            index,
            Box::new(move |_, summary| {
                let _ = tx.send(summary.hashed);
            }),
            std::time::Duration::from_secs(60),
        );
        let job = Job {
            roots: vec![root.clone()],
            disabled: Vec::new(),
        };
        let wait = |ms| rx.recv_timeout(std::time::Duration::from_millis(ms));
        service.request(job.clone());
        assert_eq!(wait(5000), Ok(1));
        service.request(job.clone());
        assert!(wait(300).is_err(), "a plain request waits out the spacing");
        // A file appears: the waiting pass starts now and finds it.
        std::fs::write(root.join("b.jpg"), b"b").unwrap();
        service.request_soon(job);
        assert_eq!(wait(5000), Ok(1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_pass_hashes_only_new_or_changed_files_and_skips_disabled_folders() {
        let root = temp_dir("pass");
        std::fs::create_dir_all(root.join("off")).unwrap();
        std::fs::write(root.join("a.jpg"), b"a").unwrap();
        std::fs::write(root.join("b.png"), b"b").unwrap();
        std::fs::write(root.join("off/c.jpg"), b"c").unwrap();
        let index = LibraryIndex::open_in_memory().unwrap();
        let disabled = vec![root.join("off")];
        let never = || false;
        let first = run_pass(
            &index,
            std::slice::from_ref(&root),
            &disabled,
            &[],
            &never,
            &|_| {},
        )
        .unwrap();
        assert_eq!((first.files, first.hashed), (2, 2));
        assert!(index
            .cached_fingerprints()
            .unwrap()
            .keys()
            .all(|p| !p.contains("/off/")));

        let again = run_pass(
            &index,
            std::slice::from_ref(&root),
            &disabled,
            &[],
            &never,
            &|_| {},
        )
        .unwrap();
        assert_eq!(again.hashed, 0, "unchanged files are not read again");

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(root.join("b.png"), b"changed").unwrap();
        let changed = run_pass(
            &index,
            std::slice::from_ref(&root),
            &disabled,
            &[],
            &never,
            &|_| {},
        )
        .unwrap();
        assert_eq!(changed.hashed, 1);
        let expected = hash_file(&root.join("b.png")).unwrap();
        assert_eq!(
            index.cached_fingerprints().unwrap()[root.join("b.png").to_str().unwrap()].2,
            expected
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
