pub mod comparison;
pub mod detector;
pub mod runner;

#[derive(Clone, Copy, PartialEq, Eq)]
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
}

pub use comparison::BeforeAfterViewer;
pub use detector::UpscaleDetector;
#[allow(unused_imports)]
pub use runner::UpscaleRunner;
