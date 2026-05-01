use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static LOGGER: OnceLock<Option<BenchLogger>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
const DEFAULT_LOG_FILE_LIMIT: usize = 20;

#[derive(Clone)]
struct BenchLogger {
    tx: mpsc::SyncSender<Value>,
}

pub fn init() {
    let _ = START.set(Instant::now());
    let enabled = std::env::var("SHARPR_BENCH")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "off"))
        .unwrap_or(true);

    let _ = LOGGER.set(if enabled { BenchLogger::spawn() } else { None });

    event("app.start", json!({}));
}

pub fn event(name: &str, fields: Value) {
    let Some(Some(logger)) = LOGGER.get() else {
        return;
    };

    let record = json!({
        "ts_ms": elapsed_ms(),
        "unix_ms": unix_ms(),
        "thread": thread::current().name().unwrap_or("unnamed"),
        "event": name,
        "fields": fields,
    });

    let _ = logger.tx.try_send(record);
}

pub fn enabled() -> bool {
    matches!(LOGGER.get(), Some(Some(_)))
}

pub fn duration_ms(start: Instant) -> u128 {
    start.elapsed().as_millis()
}

#[macro_export]
macro_rules! bench_event {
    ($name:expr, $fields:expr $(,)?) => {{
        if $crate::bench::enabled() {
            $crate::bench::event($name, $fields);
        }
    }};
}

impl BenchLogger {
    fn spawn() -> Option<Self> {
        let path = log_path();
        let file = open_log_file(&path)?;
        let (tx, rx) = mpsc::sync_channel::<Value>(16_384);

        thread::Builder::new()
            .name("bench-log-writer".to_string())
            .spawn(move || {
                let mut writer = BufWriter::new(file);
                for record in rx {
                    if serde_json::to_writer(&mut writer, &record).is_ok() {
                        let _ = writer.write_all(b"\n");
                        let _ = writer.flush();
                    }
                }
            })
            .ok()?;

        let logger = Self { tx };
        logger.spawn_sampler();
        event_with_logger(
            &logger,
            "bench.enabled",
            json!({
                "path": path.display().to_string(),
                "sample_interval_ms": 500,
            }),
        );
        Some(logger)
    }

    fn spawn_sampler(&self) {
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("bench-sampler".to_string())
            .spawn(move || {
                let mut previous = ProcessSample::read();
                loop {
                    thread::sleep(Duration::from_millis(500));
                    let current = ProcessSample::read();
                    let cpu_pct = previous
                        .as_ref()
                        .zip(current.as_ref())
                        .and_then(|(prev, cur)| cur.cpu_percent_since(prev));

                    if let Some(sample) = &current {
                        let record = json!({
                            "ts_ms": elapsed_ms(),
                            "unix_ms": unix_ms(),
                            "thread": "bench-sampler",
                            "event": "process.sample",
                            "fields": {
                                "rss_kb": sample.rss_kb,
                                "vm_size_kb": sample.vm_size_kb,
                                "cpu_jiffies": sample.proc_jiffies,
                                "system_jiffies": sample.system_jiffies,
                                "cpu_percent": cpu_pct,
                            },
                        });
                        let _ = tx.try_send(record);
                    }
                    previous = current;
                }
            })
            .ok();
    }
}

fn event_with_logger(logger: &BenchLogger, name: &str, fields: Value) {
    let record = json!({
        "ts_ms": elapsed_ms(),
        "unix_ms": unix_ms(),
        "thread": thread::current().name().unwrap_or("unnamed"),
        "event": name,
        "fields": fields,
    });
    let _ = logger.tx.try_send(record);
}

fn log_path() -> PathBuf {
    std::env::var_os("SHARPR_BENCH_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(default_log_path)
}

fn open_log_file(path: &PathBuf) -> Option<File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        trim_old_log_files(parent);
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .ok()
}

fn default_log_path() -> PathBuf {
    let base_dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    let log_dir = base_dir.join("sharpr").join("logs");
    let session = format!("run-{}-{}.jsonl", unix_ms(), std::process::id());
    log_dir.join(session)
}

fn trim_old_log_files(dir: &std::path::Path) {
    let Ok(limit) = std::env::var("SHARPR_BENCH_LOG_LIMIT")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()
    else {
        return;
    };
    let limit = limit.unwrap_or(DEFAULT_LOG_FILE_LIMIT);
    if limit == 0 {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut files: Vec<_> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let file_name = path.file_name()?.to_str()?;
            if !(file_name.starts_with("run-") && file_name.ends_with(".jsonl")) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();

    if files.len() < limit {
        return;
    }

    files.sort_by_key(|(modified, _)| *modified);
    let remove_count = files.len().saturating_sub(limit.saturating_sub(1));
    for (_, path) in files.into_iter().take(remove_count) {
        let _ = std::fs::remove_file(path);
    }
}

fn elapsed_ms() -> u128 {
    START
        .get()
        .map(|start| start.elapsed().as_millis())
        .unwrap_or(0)
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

struct ProcessSample {
    rss_kb: Option<u64>,
    vm_size_kb: Option<u64>,
    proc_jiffies: u64,
    system_jiffies: u64,
}

impl ProcessSample {
    fn read() -> Option<Self> {
        let (rss_kb, vm_size_kb) = read_status_memory();
        Some(Self {
            rss_kb,
            vm_size_kb,
            proc_jiffies: read_process_jiffies()?,
            system_jiffies: read_system_jiffies()?,
        })
    }

    fn cpu_percent_since(&self, previous: &Self) -> Option<f64> {
        let proc_delta = self.proc_jiffies.checked_sub(previous.proc_jiffies)?;
        let system_delta = self.system_jiffies.checked_sub(previous.system_jiffies)?;
        if system_delta == 0 {
            return None;
        }
        let cpu_count = std::thread::available_parallelism()
            .map(|count| count.get() as f64)
            .unwrap_or(1.0);
        Some((proc_delta as f64 / system_delta as f64) * cpu_count * 100.0)
    }
}

fn read_status_memory() -> (Option<u64>, Option<u64>) {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    let mut rss_kb = None;
    let mut vm_size_kb = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            rss_kb = parse_kb_value(value);
        } else if let Some(value) = line.strip_prefix("VmSize:") {
            vm_size_kb = parse_kb_value(value);
        }
    }
    (rss_kb, vm_size_kb)
}

fn parse_kb_value(value: &str) -> Option<u64> {
    value.split_whitespace().next()?.parse().ok()
}

fn read_process_jiffies() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after_name = stat.rsplit_once(") ")?.1;
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

fn read_system_jiffies() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let cpu_line = stat.lines().next()?;
    cpu_line
        .split_whitespace()
        .skip(1)
        .filter_map(|field| field.parse::<u64>().ok())
        .reduce(|sum, value| sum + value)
}
