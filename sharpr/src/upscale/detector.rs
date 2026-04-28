use std::path::PathBuf;

/// Locates a supported Vulkan upscaler binary in Flatpak-aware search paths.
pub struct UpscaleDetector;

impl UpscaleDetector {
    /// Returns the path to the preferred Vulkan backend if found, `None` otherwise.
    pub fn find_realesrgan() -> Option<PathBuf> {
        let candidates = Self::search_paths();
        for dir in candidates {
            for name in ["upscayl-bin", "realesrgan-ncnn-vulkan"] {
                let binary = dir.join(name);
                if binary.is_file() {
                    return Some(binary);
                }
            }
            for name in ["upscayl-bin-v0", "realesrgan-ncnn-vulkan-v0"] {
                let binary_versioned = dir.join(name);
                if binary_versioned.is_file() {
                    return Some(binary_versioned);
                }
            }
        }

        // Fall back to PATH search.
        which_binary("upscayl-bin").or_else(|| which_binary("realesrgan-ncnn-vulkan"))
    }

    fn search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Flatpak bundled location (set by our manifest).
        if let Ok(app_data) = std::env::var("FLATPAK_ID") {
            let _ = app_data; // app id present → we're inside Flatpak
            paths.push(PathBuf::from("/app/bin"));
            paths.push(PathBuf::from("/app/tools"));
        }

        // User-local installs.
        if let Some(home) = dirs_home() {
            paths.push(home.join(".local/bin"));
        }

        // System paths.
        paths.push(PathBuf::from("/usr/local/bin"));
        paths.push(PathBuf::from("/usr/bin"));

        paths
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

fn which_binary(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("sharpr-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_supported_binary_on_path() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("which-binary");
        let home = temp_dir("which-home");
        let binary = dir.join("upscayl-bin");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

        let original = std::env::var_os("PATH");
        let original_home = std::env::var_os("HOME");
        std::env::set_var("PATH", &dir);
        std::env::set_var("HOME", &home);
        std::env::remove_var("FLATPAK_ID");

        let found = UpscaleDetector::find_realesrgan();

        match original {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_file(&binary);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(found, Some(binary));
    }
}
