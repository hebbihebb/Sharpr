use std::path::PathBuf;

/// Locates the realesrgan-ncnn-vulkan binary in Flatpak-aware search paths.
pub struct UpscaleDetector;

impl UpscaleDetector {
    /// Returns the path to `realesrgan-ncnn-vulkan` if found, `None` otherwise.
    pub fn find_realesrgan() -> Option<PathBuf> {
        let candidates = Self::search_paths();
        for dir in candidates {
            let binary = dir.join("realesrgan-ncnn-vulkan");
            if binary.is_file() {
                return Some(binary);
            }
            // Some distributions ship it with a version suffix.
            let binary_versioned = dir.join("realesrgan-ncnn-vulkan-v0");
            if binary_versioned.is_file() {
                return Some(binary_versioned);
            }
        }

        // Fall back to PATH search.
        which_binary("realesrgan-ncnn-vulkan")
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
