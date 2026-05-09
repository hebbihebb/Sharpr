use crate::metadata::ImageMetadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QualityClass {
    Excellent,
    Good,
    Fair,
    Poor,
    NeedsUpscale,
}

impl QualityClass {
    pub const ALL: [Self; 5] = [
        Self::NeedsUpscale,
        Self::Poor,
        Self::Fair,
        Self::Good,
        Self::Excellent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NeedsUpscale => "720p or lower",
            Self::Poor => "900p",
            Self::Fair => "1080p",
            Self::Good => "1440p",
            Self::Excellent => "4K+",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "720p or lower" => Some(Self::NeedsUpscale),
            "900p" => Some(Self::Poor),
            "1080p" => Some(Self::Fair),
            "1440p" => Some(Self::Good),
            "4K+" => Some(Self::Excellent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityScore {
    pub score: u8,
    pub class: QualityClass,
    pub reason: String,
}

impl QualityScore {
    pub fn tooltip(&self) -> String {
        format!(
            "Resolution: {}% • {}\n{}",
            self.score,
            self.class.label(),
            self.reason
        )
    }
}

pub fn score_metadata(meta: &ImageMetadata) -> QualityScore {
    score_file_info(
        Some((meta.width, meta.height)).filter(|(width, height)| *width > 0 && *height > 0),
    )
}

pub fn score_file_info(dimensions: Option<(u32, u32)>) -> QualityScore {
    score_dimensions(dimensions)
}

fn score_dimensions(dimensions: Option<(u32, u32)>) -> QualityScore {
    let Some((width, height)) = dimensions else {
        return QualityScore {
            score: 0,
            class: QualityClass::NeedsUpscale,
            reason: "Missing image dimensions".to_string(),
        };
    };

    if width == 0 || height == 0 {
        return QualityScore {
            score: 0,
            class: QualityClass::NeedsUpscale,
            reason: "Missing image dimensions".to_string(),
        };
    }

    let short_edge = width.min(height);
    let class = classify_short_edge(short_edge);
    let score = score_short_edge(short_edge);
    let reason = build_reason(width, height, short_edge);

    QualityScore {
        score,
        class,
        reason,
    }
}

fn classify_short_edge(short_edge: u32) -> QualityClass {
    match short_edge {
        0..=720 => QualityClass::NeedsUpscale,
        721..=900 => QualityClass::Poor,
        901..=1080 => QualityClass::Fair,
        1081..=1440 => QualityClass::Good,
        _ => QualityClass::Excellent,
    }
}

fn score_short_edge(short_edge: u32) -> u8 {
    match classify_short_edge(short_edge) {
        QualityClass::NeedsUpscale => 20,
        QualityClass::Poor => 40,
        QualityClass::Fair => 60,
        QualityClass::Good => 80,
        QualityClass::Excellent => 100,
    }
}

fn build_reason(width: u32, height: u32, short_edge: u32) -> String {
    format!("{width} × {height}; short edge {short_edge}px")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score_raw(width: u32, height: u32) -> QualityScore {
        score_dimensions(Some((width, height)))
    }

    #[test]
    fn labels_match_resolution_tiers() {
        assert_eq!(QualityClass::NeedsUpscale.label(), "720p or lower");
        assert_eq!(QualityClass::Poor.label(), "900p");
        assert_eq!(QualityClass::Fair.label(), "1080p");
        assert_eq!(QualityClass::Good.label(), "1440p");
        assert_eq!(QualityClass::Excellent.label(), "4K+");
    }

    #[test]
    fn short_edge_boundaries_match_resolution_tiers() {
        assert_eq!(score_raw(1280, 720).class, QualityClass::NeedsUpscale);
        assert_eq!(score_raw(1281, 721).class, QualityClass::Poor);
        assert_eq!(score_raw(1600, 900).class, QualityClass::Poor);
        assert_eq!(score_raw(1601, 901).class, QualityClass::Fair);
        assert_eq!(score_raw(1920, 1080).class, QualityClass::Fair);
        assert_eq!(score_raw(1921, 1081).class, QualityClass::Good);
        assert_eq!(score_raw(2560, 1440).class, QualityClass::Good);
        assert_eq!(score_raw(2561, 1441).class, QualityClass::Excellent);
    }

    #[test]
    fn portrait_and_landscape_share_resolution_tier() {
        assert_eq!(score_raw(1920, 1080).class, QualityClass::Fair);
        assert_eq!(score_raw(1080, 1920).class, QualityClass::Fair);
        assert_eq!(score_raw(3840, 2160).class, QualityClass::Excellent);
        assert_eq!(score_raw(2160, 3840).class, QualityClass::Excellent);
    }

    #[test]
    fn missing_dimensions_use_lowest_tier() {
        let score = score_dimensions(None);
        assert_eq!(score.score, 0);
        assert_eq!(score.class, QualityClass::NeedsUpscale);
    }
}
