//! Batch WebP export (`ROADMAP.md` Phase 6, decision D13): the selection,
//! as WebP files in one folder, lossless by default, optionally fitted
//! within a box that only ever shrinks images.
//!
//! Originals are only read. Each output gets a new, unique name
//! (`name.webp`, then `name-1.webp`, …) that is claimed exclusively before
//! anything is written, the data goes to a hidden partial file first, and a
//! cancelled or failed image leaves nothing behind.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;

use super::{decode_oriented, reserve_output, webp_bytes, write_flushed, WEBP_MAX_DIMENSION};

#[derive(Clone, Debug, PartialEq)]
pub struct WebpOptions {
    pub destination: PathBuf,
    /// Lossless keeps every pixel; otherwise `quality` (1–100) applies.
    pub lossless: bool,
    pub quality: u8,
    /// Shrink to fit within this width × height, keeping the aspect ratio.
    /// Smaller images are never enlarged.
    pub fit: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    pub written: Vec<PathBuf>,
    /// Each source that failed, with the reason.
    pub failed: Vec<(PathBuf, String)>,
    pub canceled: bool,
}

/// The size `width`×`height` takes when fitted within `bounds`: the same
/// aspect ratio, never larger than it was.
pub fn fit_within((width, height): (u32, u32), (max_w, max_h): (u32, u32)) -> (u32, u32) {
    if width <= max_w && height <= max_h {
        return (width, height);
    }
    let scale = (f64::from(max_w) / f64::from(width)).min(f64::from(max_h) / f64::from(height));
    (
        ((f64::from(width) * scale).round() as u32).clamp(1, max_w),
        ((f64::from(height) * scale).round() as u32).clamp(1, max_h),
    )
}

/// Exports `sources` one after another. `progress(done, total)` follows
/// each image; `canceled` is checked between images and before an image's
/// output is kept, so cancelling never leaves a partial file.
pub fn export_webp(
    sources: &[PathBuf],
    options: &WebpOptions,
    canceled: &dyn Fn() -> bool,
    progress: &dyn Fn(usize, usize),
) -> BatchReport {
    let mut report = BatchReport::default();
    if let Err(error) = std::fs::create_dir_all(&options.destination) {
        for source in sources {
            report.failed.push((source.clone(), format!("{error}")));
        }
        return report;
    }
    for (i, source) in sources.iter().enumerate() {
        if canceled() {
            report.canceled = true;
            break;
        }
        match export_one(source, options, canceled) {
            Ok(Some(output)) => report.written.push(output),
            Ok(None) => {
                report.canceled = true;
                break;
            }
            Err(error) => report.failed.push((source.clone(), error)),
        }
        progress(i + 1, sources.len());
    }
    crate::bench_event!(
        "export.webp.batch",
        serde_json::json!({
            "sources": sources.len(),
            "written": report.written.len(),
            "failed": report.failed.len(),
            "canceled": report.canceled,
            "lossless": options.lossless,
        }),
    );
    report
}

/// One image; `Ok(None)` when cancelled before its output was kept.
fn export_one(
    source: &Path,
    options: &WebpOptions,
    canceled: &dyn Fn() -> bool,
) -> Result<Option<PathBuf>, String> {
    let image = decode_oriented(source).map_err(|e| e.to_string())?;
    let size = (image.width(), image.height());
    let target = options
        .fit
        .map(|bounds| fit_within(size, bounds))
        .unwrap_or(size);
    if target.0 > WEBP_MAX_DIMENSION || target.1 > WEBP_MAX_DIMENSION {
        return Err(format!(
            "{}×{} is larger than WebP allows ({WEBP_MAX_DIMENSION} pixels per side); \
             export it with Fit Within",
            target.0, target.1
        ));
    }
    let image = if target == size {
        image
    } else {
        image.resize_exact(target.0, target.1, FilterType::Lanczos3)
    };
    let encoded =
        webp_bytes(&image, options.lossless, options.quality).map_err(|e| e.to_string())?;
    drop(image);

    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("image")
        .to_string();
    let (output, partial) = reserve_output(|suffix| {
        options.destination.join(if suffix == 0 {
            format!("{stem}.webp")
        } else {
            format!("{stem}-{suffix}.webp")
        })
    })
    .map_err(|e| e.to_string())?;
    let discard = || {
        let _ = std::fs::remove_file(&partial);
        let _ = std::fs::remove_file(&output);
    };
    let written = write_flushed(&partial, &encoded);
    if let Err(error) = written {
        discard();
        return Err(format!("writing {}: {error}", partial.display()));
    }
    if canceled() {
        discard();
        return Ok(None);
    }
    if let Err(error) = std::fs::rename(&partial, &output) {
        discard();
        return Err(format!("moving into place {}: {error}", output.display()));
    }
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sharpr-batch-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pattern(width: u32, height: u32) -> image::RgbImage {
        image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
        })
    }

    fn options(destination: &Path) -> WebpOptions {
        WebpOptions {
            destination: destination.to_path_buf(),
            lossless: true,
            quality: 90,
            fit: None,
        }
    }

    #[test]
    fn fitting_only_shrinks_and_keeps_the_aspect_ratio() {
        assert_eq!(fit_within((7680, 4320), (3840, 2160)), (3840, 2160));
        assert_eq!(fit_within((4000, 2000), (3840, 2160)), (3840, 1920));
        assert_eq!(fit_within((3000, 6000), (3840, 2160)), (1080, 2160));
        assert_eq!(fit_within((1920, 1080), (3840, 2160)), (1920, 1080));
    }

    #[test]
    fn lossless_export_keeps_every_pixel_and_never_touches_the_original() {
        let dir = temp_dir("lossless");
        let source = dir.join("photo.png");
        pattern(64, 40).save(&source).unwrap();
        let before = std::fs::read(&source).unwrap();
        let out = dir.join("out");
        let report = export_webp(
            std::slice::from_ref(&source),
            &options(&out),
            &|| false,
            &|_, _| {},
        );
        assert_eq!(report.written, vec![out.join("photo.webp")]);
        let exported = image::open(&report.written[0]).unwrap().to_rgb8();
        assert_eq!(exported, pattern(64, 40), "lossless");
        assert_eq!(
            std::fs::read(&source).unwrap(),
            before,
            "original untouched"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn exports_fit_get_unique_names_and_report_failures() {
        let dir = temp_dir("names");
        let big = dir.join("big.png");
        pattern(400, 200).save(&big).unwrap();
        let broken = dir.join("broken.png");
        std::fs::write(&broken, b"not an image").unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("big.webp"), b"someone else's file").unwrap();
        let mut opts = options(&out);
        opts.fit = Some((100, 100));
        opts.lossless = false;
        let report = export_webp(&[big.clone(), broken.clone()], &opts, &|| false, &|_, _| {});
        assert_eq!(
            report.written,
            vec![out.join("big-1.webp")],
            "never overwrites"
        );
        assert_eq!(
            std::fs::read(out.join("big.webp")).unwrap(),
            b"someone else's file"
        );
        let exported = image::open(out.join("big-1.webp")).unwrap();
        assert_eq!((exported.width(), exported.height()), (100, 50));
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, broken);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelling_keeps_finished_images_and_leaves_no_partial_files() {
        let dir = temp_dir("cancel");
        let sources: Vec<PathBuf> = (0..3)
            .map(|i| {
                let path = dir.join(format!("{i}.png"));
                pattern(32, 32).save(&path).unwrap();
                path
            })
            .collect();
        let out = dir.join("out");
        let done = std::cell::Cell::new(0);
        let report = export_webp(
            &sources,
            &options(&out),
            &|| done.get() >= 1,
            &|finished, _| done.set(finished),
        );
        assert!(report.canceled);
        assert_eq!(report.written, vec![out.join("0.webp")]);
        let names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["0.webp".to_string()],
            "no partial or empty files"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
