//! Where the model and its runtime are installed (decision D23).
//!
//! The user installs the files once; Sharpr only looks for them and never
//! downloads anything. The folder is separate from `~/.local/share/sharpr/`,
//! the irreplaceable data directory.

use std::path::{Path, PathBuf};

pub const MODEL_FILE: &str = "embeddinggemma-2-Q8_0.gguf";
pub const PROJECTOR_FILE: &str = "mmproj-embeddinggemma-2-Q8_0.gguf";

/// Installed model files and the `llama-server` that runs them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFiles {
    pub model: PathBuf,
    pub projector: PathBuf,
    pub server: PathBuf,
    /// A GPU (Vulkan) build of `llama-server` for indexing, if installed in
    /// the model folder's `runtime-gpu/`.
    pub gpu_server: Option<PathBuf>,
}

/// The folder Sharpr checks for the model:
/// `~/.local/share/sharpr-models/embeddinggemma-2/`, inside the sandbox's own
/// data folder under Flatpak.
pub fn model_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sharpr-models")
        .join("embeddinggemma-2")
}

/// The Flatpak's own builds of `llama-server`, with their libraries beside
/// them: the processor build and the Vulkan build (D34).
const FLATPAK_SERVER: &str = "/app/libexec/sharpr-llama/llama-server";
const FLATPAK_GPU_SERVER: &str = "/app/libexec/sharpr-llama-gpu/llama-server";

/// Where `llama-server` may be, in order: an explicit override, the
/// Flatpak's own build, and the model folder's `runtime/` (native installs).
fn server_candidates(dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("SHARPR_LLAMA_SERVER") {
        candidates.push(PathBuf::from(path));
    }
    candidates.push(PathBuf::from(FLATPAK_SERVER));
    candidates.push(dir.join("runtime").join("llama-server"));
    candidates
}

/// Finds the installed files, or explains in plain words what is missing.
/// Under Flatpak, a model installed for the native app in the home folder
/// (reachable through `--filesystem=home`) is used when the sandbox's own
/// folder has none, so one copy serves both.
pub fn locate() -> Result<ModelFiles, String> {
    let dir = model_dir();
    let found = locate_in(&dir, &server_candidates(&dir));
    if found.is_ok() || std::env::var_os("FLATPAK_ID").is_none() {
        return found;
    }
    let host = dirs::home_dir()
        .map(|home| home.join(".local/share/sharpr-models/embeddinggemma-2"))
        .filter(|host| *host != dir);
    match host {
        Some(host) => locate_in(&host, &server_candidates(&host)).or(found),
        None => found,
    }
}

pub fn locate_in(dir: &Path, servers: &[PathBuf]) -> Result<ModelFiles, String> {
    let model = dir.join(MODEL_FILE);
    let projector = dir.join(PROJECTOR_FILE);
    let missing: Vec<&str> = [(&model, MODEL_FILE), (&projector, PROJECTOR_FILE)]
        .into_iter()
        .filter(|(path, _)| !path.is_file())
        .map(|(_, name)| name)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Visual search needs EmbeddingGemma 2. Missing from {}: {}",
            dir.display(),
            missing.join(", ")
        ));
    }
    for path in [&model, &projector] {
        if !is_gguf(path) {
            return Err(format!(
                "{} is not a GGUF model file; reinstall EmbeddingGemma 2",
                path.display()
            ));
        }
    }
    let server = servers
        .iter()
        .find(|path| is_executable(path))
        .cloned()
        .ok_or_else(|| {
            format!(
                "Visual search needs llama-server. Put it in {}",
                dir.join("runtime").display()
            )
        })?;
    let gpu_server = std::env::var_os("SHARPR_LLAMA_SERVER_GPU")
        .map(PathBuf::from)
        .into_iter()
        .chain([
            PathBuf::from(FLATPAK_GPU_SERVER),
            dir.join("runtime-gpu").join("llama-server"),
        ])
        .find(|path| is_executable(path));
    Ok(ModelFiles {
        model,
        projector,
        server,
        gpu_server,
    })
}

fn is_gguf(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut magic))
        .is_ok()
        && &magic == b"GGUF"
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_wrong_files_are_explained() {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-model-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        let server = dir.join("runtime/llama-server");
        let servers = vec![server.clone()];

        let error = locate_in(&dir, &servers).unwrap_err();
        assert!(
            error.contains(MODEL_FILE) && error.contains(PROJECTOR_FILE),
            "{error}"
        );

        std::fs::write(dir.join(MODEL_FILE), b"GGUF....").unwrap();
        std::fs::write(dir.join(PROJECTOR_FILE), b"<html>").unwrap();
        let error = locate_in(&dir, &servers).unwrap_err();
        assert!(error.contains("not a GGUF"), "{error}");

        std::fs::write(dir.join(PROJECTOR_FILE), b"GGUF....").unwrap();
        let error = locate_in(&dir, &servers).unwrap_err();
        assert!(error.contains("llama-server"), "{error}");

        std::fs::write(&server, b"#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        let files = locate_in(&dir, &servers).unwrap();
        assert_eq!(files.server, server);
        assert_eq!(files.model, dir.join(MODEL_FILE));

        std::fs::remove_dir_all(dir).unwrap();
    }
}
