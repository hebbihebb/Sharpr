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
