/// Stub for Phase 5 — AI upscaling subprocess runner.
///
/// Will wrap `gio::Subprocess` invocation of realesrgan-ncnn-vulkan
/// and stream stderr progress updates back to the UI thread.
pub struct UpscaleRunner;

impl UpscaleRunner {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UpscaleRunner {
    fn default() -> Self {
        Self::new()
    }
}
