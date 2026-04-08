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
        Self::Excellent,
        Self::Good,
        Self::Fair,
        Self::Poor,
        Self::NeedsUpscale,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Excellent => "Excellent",
            Self::Good => "Good",
            Self::Fair => "Fair",
            Self::Poor => "Poor",
            Self::NeedsUpscale => "Needs Upscale",
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
            "IQ: {}% • {}\n{}",
            self.score,
            self.class.label(),
            self.reason
        )
    }
}

pub fn score_metadata(meta: &ImageMetadata) -> QualityScore {
    score_file_info(
        Some((meta.width, meta.height)).filter(|(width, height)| *width > 0 && *height > 0),
        meta.file_size_bytes,
        &normalize_format(&meta.format),
    )
}

pub fn score_file_info(
    dimensions: Option<(u32, u32)>,
    file_size_bytes: u64,
    format: &str,
) -> QualityScore {
    score_dimensions(dimensions, file_size_bytes, normalize_format(format))
}

fn score_dimensions(
    dimensions: Option<(u32, u32)>,
    file_size_bytes: u64,
    format: String,
) -> QualityScore {
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

    let pixels = (width as f64) * (height as f64);
    let megapixels = pixels / 1_000_000.0;
    let long_edge = width.max(height);
    let bpp = if pixels > 0.0 {
        file_size_bytes as f64 / pixels
    } else {
        0.0
    };

    let long_edge_score = score_long_edge(long_edge);
    let megapixel_score = score_megapixels(megapixels);
    let detail_score = ((long_edge_score as f64 * 0.65) + (megapixel_score as f64 * 0.35)).round();
    let compression_score = score_compression(&format, bpp);
    let format_score = score_format(&format);

    let total =
        ((detail_score * 0.5) + (compression_score as f64 * 0.3) + (format_score as f64 * 0.2))
            .round()
            .clamp(0.0, 100.0) as u8;
    let class = classify(total);
    let reason = build_reason(long_edge, detail_score as u8, compression_score, &format);

    QualityScore {
        score: total,
        class,
        reason,
    }
}

fn score_long_edge(long_edge: u32) -> u8 {
    match long_edge {
        3840.. => 100,
        3200..=3839 => 92,
        2560..=3199 => 82,
        1920..=2559 => 70,
        1600..=1919 => 56,
        1280..=1599 => 42,
        1024..=1279 => 28,
        _ => 14,
    }
}

fn score_megapixels(megapixels: f64) -> u8 {
    match megapixels {
        mp if mp >= 12.0 => 100,
        mp if mp >= 8.0 => 92,
        mp if mp >= 5.0 => 80,
        mp if mp >= 3.0 => 64,
        mp if mp >= 2.0 => 50,
        mp if mp >= 1.0 => 34,
        _ => 18,
    }
}

fn score_compression(format: &str, bpp: f64) -> u8 {
    let (floor, target) = match format {
        "AVIF" | "HEIC" | "HEIF" => (0.08, 0.30),
        "WEBP" => (0.10, 0.42),
        "PNG" => (0.15, 1.10),
        "TIFF" | "TIF" => (0.20, 1.40),
        "BMP" => (0.20, 1.50),
        "GIF" => (0.06, 0.20),
        _ => (0.12, 0.72),
    };

    if bpp <= floor {
        return 18;
    }
    if bpp >= target {
        return 100;
    }

    (((bpp - floor) / (target - floor)) * 82.0 + 18.0)
        .round()
        .clamp(0.0, 100.0) as u8
}

fn score_format(format: &str) -> u8 {
    match format {
        "PNG" | "TIFF" | "TIF" => 96,
        "AVIF" | "HEIC" | "HEIF" => 94,
        "WEBP" => 90,
        "JPEG" | "JPG" => 76,
        "BMP" => 68,
        "GIF" | "ICO" => 30,
        _ => 60,
    }
}

fn classify(score: u8) -> QualityClass {
    match score {
        85..=100 => QualityClass::Excellent,
        70..=84 => QualityClass::Good,
        50..=69 => QualityClass::Fair,
        30..=49 => QualityClass::Poor,
        _ => QualityClass::NeedsUpscale,
    }
}

fn build_reason(long_edge: u32, detail_score: u8, compression_score: u8, format: &str) -> String {
    if long_edge < 1920 {
        return "Low resolution for wallpaper use".to_string();
    }

    if detail_score >= 88 && compression_score >= 74 {
        if matches!(
            format,
            "AVIF" | "HEIC" | "HEIF" | "WEBP" | "PNG" | "TIFF" | "TIF"
        ) {
            return "High resolution, efficient format".to_string();
        }
        return "High resolution, lightly compressed".to_string();
    }

    if compression_score < 40 {
        return "Good resolution, heavy compression".to_string();
    }

    if detail_score >= 68 {
        return "Good resolution, moderate compression".to_string();
    }

    "Moderate resolution, moderate compression".to_string()
}

fn normalize_format(format: &str) -> String {
    format.trim().to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score_raw(width: u32, height: u32, file_size_bytes: u64, format: &str) -> QualityScore {
        score_dimensions(Some((width, height)), file_size_bytes, format.to_string())
    }

    #[test]
    fn class_boundaries_match_product_ranges() {
        assert_eq!(classify(85), QualityClass::Excellent);
        assert_eq!(classify(70), QualityClass::Good);
        assert_eq!(classify(50), QualityClass::Fair);
        assert_eq!(classify(30), QualityClass::Poor);
        assert_eq!(classify(29), QualityClass::NeedsUpscale);
    }

    #[test]
    fn high_resolution_efficient_image_scores_high() {
        let score = score_raw(3840, 2160, 2_900_000, "AVIF");
        assert!(score.score >= 85, "score was {}", score.score);
        assert_eq!(score.class, QualityClass::Excellent);
    }

    #[test]
    fn low_resolution_small_jpeg_needs_upscale() {
        let score = score_raw(960, 640, 65_000, "JPEG");
        assert!(score.score < 30, "score was {}", score.score);
        assert_eq!(score.class, QualityClass::NeedsUpscale);
    }

    #[test]
    fn moderate_wallpaper_jpeg_lands_in_middle_bands() {
        let score = score_raw(1920, 1080, 480_000, "JPEG");
        assert!(
            (45..=84).contains(&score.score),
            "score was {}",
            score.score
        );
        assert!(matches!(
            score.class,
            QualityClass::Fair | QualityClass::Good
        ));
    }
}
