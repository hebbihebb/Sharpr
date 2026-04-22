pub mod backend;
pub mod backends;
pub mod comparison;
pub mod detector;
pub mod downloader;
pub mod runner;
pub mod tiling;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscaleModel {
    Standard,
    Anime,
}

impl UpscaleModel {
    pub fn from_settings(value: &str) -> Self {
        match value {
            "anime" => Self::Anime,
            _ => Self::Standard,
        }
    }

    pub fn settings_key(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Anime => "anime",
        }
    }

    pub fn model_name(self) -> &'static str {
        match self {
            Self::Standard => "realesrgan-x4plus",
            Self::Anime => "realesrgan-x4plus-anime",
        }
    }

    pub fn native_scale(self) -> u32 {
        match self {
            Self::Standard | Self::Anime => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscaleOutputFormat {
    Auto,
    Jpeg,
    Png,
    Webp,
}

impl UpscaleOutputFormat {
    pub fn from_settings(value: &str) -> Self {
        match value {
            "jpeg" => Self::Jpeg,
            "png" => Self::Png,
            "webp" => Self::Webp,
            _ => Self::Auto,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Auto => "png",
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscaleCompressionMode {
    Auto,
    Lossy,
    Lossless,
}

impl UpscaleCompressionMode {
    pub fn from_settings(value: &str) -> Self {
        match value {
            "lossy" => Self::Lossy,
            "lossless" => Self::Lossless,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct UpscaleJobConfig {
    pub source_dimensions: (u32, u32),
    pub requested_scale: u32,
    pub execution_scale: u32,
    pub model: UpscaleModel,
    pub output_format: UpscaleOutputFormat,
    pub compression_mode: UpscaleCompressionMode,
    pub quality: u8,
    pub tile_size: Option<u32>,
    pub gpu_id: Option<u32>,
}

/// Which upscale backend to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UpscaleBackendKind {
    #[default]
    Cli,
    Onnx,
    ComfyUi,
}

impl UpscaleBackendKind {
    pub fn from_settings(value: &str) -> Self {
        match value {
            "onnx" => Self::Onnx,
            "comfyui" => Self::ComfyUi,
            _ => Self::Cli,
        }
    }

    pub fn settings_key(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Onnx => "onnx",
            Self::ComfyUi => "comfyui",
        }
    }
}

/// Static metadata for an ONNX upscale model.
pub struct OnnxModelInfo {
    pub filename: &'static str,
    pub download_url: &'static str,
    /// Approximate download size in MiB shown in the UI.
    pub download_size_mb: u32,
    /// Scale factor this model produces (informational; not passed into inference).
    pub native_scale: usize,
    /// Input tile dimensions must be a multiple of this value.
    pub window_size: usize,
    /// Short human-readable label for the UI dropdown.
    pub display_name: &'static str,
}

/// Which ONNX model file to load for the ONNX upscale backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OnnxUpscaleModel {
    /// Swin2SR Lightweight ×2 (64-window, 8 MB) — fast, good starting point.
    #[default]
    Swin2srLightweightX2,
    /// Swin2SR CompressedSR ×4 (48-window, 55 MB) — compressed image SR.
    Swin2srCompressedX4,
    /// Swin2SR RealworldSR ×4 (64-window, 55 MB) — best quality.
    Swin2srRealX4,
}

impl OnnxUpscaleModel {
    pub fn from_settings(value: &str) -> Self {
        match value {
            "swin2sr-compressed-x4" => Self::Swin2srCompressedX4,
            "swin2sr-real-x4" => Self::Swin2srRealX4,
            _ => Self::Swin2srLightweightX2,
        }
    }

    pub fn settings_key(self) -> &'static str {
        match self {
            Self::Swin2srLightweightX2 => "swin2sr-lightweight-x2",
            Self::Swin2srCompressedX4 => "swin2sr-compressed-x4",
            Self::Swin2srRealX4 => "swin2sr-real-x4",
        }
    }

    pub fn info(self) -> OnnxModelInfo {
        match self {
            Self::Swin2srLightweightX2 => OnnxModelInfo {
                filename: "swin2sr_lightweight_x2.onnx",
                download_url: "https://huggingface.co/Xenova/swin2SR-lightweight-x2-64/resolve/main/onnx/model.onnx",
                download_size_mb: 8,
                native_scale: 2,
                window_size: 64,
                display_name: "Lightweight ×2  —  8 MB",
            },
            Self::Swin2srCompressedX4 => OnnxModelInfo {
                filename: "swin2sr_compressed_x4.onnx",
                download_url: "https://huggingface.co/Xenova/swin2SR-compressed-sr-x4-48/resolve/main/onnx/model.onnx",
                download_size_mb: 55,
                native_scale: 4,
                window_size: 48,
                display_name: "Compressed ×4  —  55 MB",
            },
            Self::Swin2srRealX4 => OnnxModelInfo {
                filename: "swin2sr_real_x4.onnx",
                // Placeholder URL — update when onnx-community repo is confirmed.
                download_url: "https://huggingface.co/Xenova/swin2SR-compressed-sr-x4-48/resolve/main/onnx/model.onnx",
                download_size_mb: 55,
                native_scale: 4,
                window_size: 64,
                display_name: "Realworld ×4  —  55 MB",
            },
        }
    }

    /// Convenience: local filename for this model.
    pub fn filename(self) -> &'static str {
        self.info().filename
    }
}

pub use backends::comfyui::ComfyUiBackend;
pub use comparison::BeforeAfterViewer;
pub use detector::UpscaleDetector;
#[allow(unused_imports)]
pub use runner::UpscaleRunner;
