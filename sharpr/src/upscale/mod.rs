pub mod comparison;
pub mod detector;
pub mod runner;

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

pub use comparison::BeforeAfterViewer;
pub use detector::UpscaleDetector;
#[allow(unused_imports)]
pub use runner::UpscaleRunner;
