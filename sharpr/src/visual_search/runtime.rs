//! `llama-server` child processes (decision D24).
//!
//! Each server listens on 127.0.0.1 at a free port and requires a random key,
//! so nothing else on the machine can use it. It is killed when its owner
//! drops it, and the kernel kills it if the thread that started it ends, so
//! a crash or quit of Sharpr never leaves one running. The HTTP client is a
//! few lines over `TcpStream`; it only ever talks to these children.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::files::ModelFiles;
use super::DIMENSIONS;

/// What a server is for. Images need the projector; queries need only the
/// text model, which starts in about half the time and gives identical
/// query vectors, so a search never waits behind an image being indexed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Images,
    Queries,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Images => "images",
            Mode::Queries => "queries",
        }
    }

    /// Indexing measured +5–7% browsing slowdown with 2 threads at nice 19
    /// (Phase 3); queries are short and want an answer now.
    fn threads(self) -> usize {
        match self {
            Mode::Images => 2,
            Mode::Queries => 4,
        }
    }
}

pub struct Server {
    child: Child,
    port: u16,
    key: String,
    mode: Mode,
}

/// How long a server may take to load its model before Sharpr gives up.
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// Long enough for one image on a slow, busy CPU.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// A graphics card `llama-server` can run on, from `--list-devices`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuDevice {
    /// The id `--device` takes, such as `Vulkan0`.
    pub id: String,
    pub name: String,
    pub free_mib: u64,
}

/// The model, projector, and working memory need about 1.2 GB on the card.
const GPU_FREE_MIB: u64 = 2048;

/// The first graphics card with room for the model. ggml lists discrete
/// cards before integrated ones, which share the computer's own memory.
pub fn pick_gpu(gpu_server: &std::path::Path) -> Option<GpuDevice> {
    let mut command = Command::new(gpu_server);
    command
        .arg("--list-devices")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    set_library_path(&mut command, gpu_server);
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    while matches!(child.try_wait(), Ok(None)) {
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    parse_devices(&text)
        .into_iter()
        .find(|device| device.free_mib >= GPU_FREE_MIB)
}

/// Lines like `  Vulkan0: NVIDIA GeForce RTX 4060 (8188 MiB, 7038 MiB free)`.
pub fn parse_devices(text: &str) -> Vec<GpuDevice> {
    text.lines()
        .filter_map(|line| {
            let (id, rest) = line.trim().split_once(": ")?;
            if id.contains(' ') || id.is_empty() {
                return None;
            }
            let open = rest.rfind(" (")?;
            let memory = rest[open + 2..].strip_suffix(" MiB free)")?;
            let free = memory.rsplit(", ").next()?.trim().parse().ok()?;
            Some(GpuDevice {
                id: id.to_string(),
                name: rest[..open].to_string(),
                free_mib: free,
            })
        })
        .collect()
}

/// The prebuilt runtimes keep their libraries beside the binary.
fn set_library_path(command: &mut Command, server: &std::path::Path) {
    if let Some(dir) = server.parent() {
        let mut paths = vec![dir.to_path_buf()];
        if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(paths) {
            command.env("LD_LIBRARY_PATH", joined);
        }
    }
}

impl Server {
    /// Starts a server on the processor and waits until it answers. The
    /// child inherits the calling thread's CPU and I/O priority.
    pub fn start(files: &ModelFiles, mode: Mode) -> Result<Self, String> {
        Self::spawn(&files.server, files, mode, None)
    }

    /// Starts an image server on `device` with the GPU build: about 17
    /// times faster than the processor, with matching vectors (Phase 5
    /// experiment, October 14).
    pub fn start_gpu(files: &ModelFiles, device: &GpuDevice) -> Result<Self, String> {
        let server = files
            .gpu_server
            .as_ref()
            .ok_or("No GPU build of llama-server is installed")?;
        Self::spawn(server, files, Mode::Images, Some(&device.id))
    }

    fn spawn(
        server_path: &std::path::Path,
        files: &ModelFiles,
        mode: Mode,
        device: Option<&str>,
    ) -> Result<Self, String> {
        let port = free_port()?;
        let key = random_key()?;
        let threads = mode.threads().to_string();
        let mut command = Command::new(server_path);
        if let Some(device) = device {
            command.args(["--device", device, "--n-gpu-layers", "99"]);
        } else {
            // A CPU build ignores this; a GPU build must not touch the card.
            command.args(["--n-gpu-layers", "0"]);
        }
        command.arg("--model").arg(&files.model);
        if mode == Mode::Images {
            command.arg("--mmproj").arg(&files.projector);
        }
        // An image is at most about 200 tokens; 2,048 leaves ample room.
        command
            .args(["--embeddings", "--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .arg("--api-key")
            .arg(&key)
            .args(["--ctx-size", "2048", "--batch-size", "2048"])
            .args(["--ubatch-size", "2048", "--parallel", "1"])
            .args(["--threads", &threads, "--threads-batch", &threads])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(server_log(mode));
        set_library_path(&mut command, server_path);
        // SAFETY: prctl is async-signal-safe and touches only the child.
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let child = command
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", server_path.display()))?;
        let mut server = Server {
            child,
            port,
            key,
            mode,
        };
        server.wait_until_ready()?;
        Ok(server)
    }

    fn wait_until_ready(&mut self) -> Result<(), String> {
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!(
                    "llama-server stopped while loading the model ({status}); see {}",
                    log_path(self.mode).display()
                ));
            }
            if matches!(
                self.request("GET", "/health", None, Duration::from_secs(2)),
                Ok((200, _))
            ) {
                return Ok(());
            }
            if started.elapsed() > START_TIMEOUT {
                return Err("llama-server did not finish loading the model in time".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The query's vector, using EmbeddingGemma's search prompt.
    pub fn embed_query(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let content = serde_json::json!([
            {"type": "text", "text": format!("{}{text}", super::QUERY_PROMPT)}
        ]);
        self.embed(content)
    }

    /// The vector of an image prepared by `prepare::prepare_png`.
    pub fn embed_png(&mut self, png: &[u8]) -> Result<Vec<f32>, String> {
        let url = format!("data:image/png;base64,{}", base64(png));
        self.embed(serde_json::json!([
            {"type": "image_url", "image_url": {"url": url}}
        ]))
    }

    fn embed(&mut self, content: serde_json::Value) -> Result<Vec<f32>, String> {
        let body = serde_json::json!({
            "model": "embeddinggemma-2",
            "input": [{"content": content}],
            "encoding_format": "float",
        });
        let body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let (status, response) =
            self.request("POST", "/v1/embeddings", Some(&body), REQUEST_TIMEOUT)?;
        if status != 200 {
            let text = String::from_utf8_lossy(&response);
            return Err(format!(
                "llama-server answered {status}: {}",
                text.chars().take(200).collect::<String>()
            ));
        }
        parse_embedding(&response)
    }

    /// Whether the process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>), String> {
        http_request(self.port, &self.key, method, path, body, timeout)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Unit-length vector from an `/v1/embeddings` response; anything that isn't
/// a finite, non-zero 768-number vector means the wrong model.
pub fn parse_embedding(body: &[u8]) -> Result<Vec<f32>, String> {
    #[derive(serde::Deserialize)]
    struct Response {
        data: Vec<Item>,
    }
    #[derive(serde::Deserialize)]
    struct Item {
        embedding: Vec<f32>,
    }
    let response: Response = serde_json::from_slice(body)
        .map_err(|e| format!("Unexpected answer from llama-server: {e}"))?;
    let mut vector = response
        .data
        .into_iter()
        .next()
        .ok_or("llama-server returned no vector")?
        .embedding;
    if vector.len() != DIMENSIONS {
        return Err(format!(
            "The installed model is not EmbeddingGemma 2: it gives {} numbers per image instead of {DIMENSIONS}",
            vector.len()
        ));
    }
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err("llama-server returned an empty vector".into());
    }
    for value in &mut vector {
        *value /= norm;
    }
    Ok(vector)
}

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .map_err(|e| format!("No free local port: {e}"))
}

fn random_key() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| format!("No random source: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn log_path(mode: Mode) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sharpr")
        .join("logs")
        .join(format!("llama-server-{}.log", mode.label()))
}

/// The server's own messages, overwritten at each start, for diagnosis.
fn server_log(mode: Mode) -> Stdio {
    let path = log_path(mode);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::File::create(&path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

/// One HTTP/1.1 exchange with a loopback server, closing the connection
/// afterwards.
pub fn http_request(
    port: u16,
    key: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<(u16, Vec<u8>), String> {
    let error = |e: std::io::Error| format!("llama-server connection: {e}");
    let mut stream =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), timeout).map_err(error)?;
    stream.set_read_timeout(Some(timeout)).map_err(error)?;
    stream.set_write_timeout(Some(timeout)).map_err(error)?;
    let body = body.unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {key}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(error)?;
    stream.write_all(body).map_err(error)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(error)?;
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("Incomplete answer from llama-server")?;
    let head = String::from_utf8_lossy(&response[..split]);
    let body = &response[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("Malformed answer from llama-server")?;
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.eq_ignore_ascii_case("chunked")
        {
            chunked = true;
        }
    }
    let body = if chunked {
        dechunk(body)?
    } else if let Some(length) = length {
        body.get(..length)
            .ok_or("Truncated answer from llama-server")?
            .to_vec()
    } else {
        body.to_vec()
    };
    Ok((status, body))
}

fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let end = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("Malformed chunked answer")?;
        let size_text = String::from_utf8_lossy(&data[..end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "Malformed chunk size")?;
        data = &data[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        out.extend_from_slice(data.get(..size).ok_or("Truncated chunk")?);
        data = data.get(size + 2..).ok_or("Truncated chunk")?;
    }
}

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_cards_are_read_from_the_device_list() {
        let text = "load_backend: loaded Vulkan backend\nAvailable devices:\n  \
            Vulkan0: NVIDIA GeForce RTX 4060 Laptop GPU (8188 MiB, 7038 MiB free)\n  \
            Vulkan1: AMD Radeon 780M Graphics (RADV PHOENIX) (8088 MiB, 8005 MiB free)\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 2);
        assert_eq!(
            devices[0],
            GpuDevice {
                id: "Vulkan0".into(),
                name: "NVIDIA GeForce RTX 4060 Laptop GPU".into(),
                free_mib: 7038
            }
        );
        assert_eq!(devices[1].name, "AMD Radeon 780M Graphics (RADV PHOENIX)");
        assert!(parse_devices("Available devices:\n").is_empty());
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn responses_parse_with_length_or_chunks() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
        assert_eq!(parse_response(plain).unwrap(), (200, b"hello".to_vec()));
        let chunked =
            b"HTTP/1.1 401 Unauthorized\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nhel\r\n2\r\nlo\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap(), (401, b"hello".to_vec()));
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort").is_err());
    }

    #[test]
    fn requests_carry_the_key_and_body() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            while !request.ends_with(b"{\"a\":1}") {
                let n = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..n]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let (status, body) = http_request(
            port,
            "secret",
            "POST",
            "/v1/embeddings",
            Some(b"{\"a\":1}"),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!((status, body), (200, b"ok".to_vec()));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/embeddings HTTP/1.1\r\n"));
        assert!(request.contains("Authorization: Bearer secret\r\n"));
        assert!(request.contains("Content-Length: 7\r\n"));
    }

    #[test]
    fn embeddings_are_normalised_and_wrong_models_rejected() {
        let mut values = vec![0.0f32; DIMENSIONS];
        values[0] = 3.0;
        values[1] = 4.0;
        let body = serde_json::json!({"data": [{"embedding": values}]}).to_string();
        let vector = parse_embedding(body.as_bytes()).unwrap();
        assert!((vector[0] - 0.6).abs() < 1e-6 && (vector[1] - 0.8).abs() < 1e-6);

        let short = serde_json::json!({"data": [{"embedding": [1.0, 2.0]}]}).to_string();
        let error = parse_embedding(short.as_bytes()).unwrap_err();
        assert!(error.contains("not EmbeddingGemma 2"), "{error}");
        let zero = serde_json::json!({"data": [{"embedding": vec![0.0; DIMENSIONS]}]}).to_string();
        assert!(parse_embedding(zero.as_bytes()).is_err());
    }
}
