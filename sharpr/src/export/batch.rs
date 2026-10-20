//! Batch export (D47): PNG, WebP or each source format in one folder,
//! PNG by default, optionally fitted
//! within a box that only ever shrinks images.
//!
//! Originals are only read. Each output gets a new, unique name
//! (`name.webp`, then `name-1.webp`, …) that is claimed exclusively before
//! anything is written, the data goes to a hidden partial file first, and a
//! cancelled or failed image leaves nothing behind.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;

use super::{decode_oriented, OutputFormat, Pending, WEBP_MAX_DIMENSION};

#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub format: OutputFormat,
    pub destination: PathBuf,
    /// For explicit WebP: lossless preserves decoded 8-bit pixels; otherwise quality applies.
    pub lossless: bool,
    pub quality: u8,
    /// Shrink to fit within this width × height, keeping the aspect ratio.
    /// Smaller images are never enlarged.
    pub fit: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    pub written: Vec<PathBuf>,
    /// The source of each `written` file, in the same order.
    pub written_from: Vec<PathBuf>,
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
pub fn export_images(
    sources: &[PathBuf],
    options: &ExportOptions,
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
            Ok(Some(output)) => {
                report.written.push(output);
                report.written_from.push(source.clone());
            }
            Ok(None) => {
                report.canceled = true;
                break;
            }
            Err(error) => report.failed.push((source.clone(), error)),
        }
        progress(i + 1, sources.len());
    }
    crate::bench_event!(
        "export.batch",
        serde_json::json!({
            "sources": sources.len(),
            "written": report.written.len(),
            "failed": report.failed.len(),
            "canceled": report.canceled,
            "lossless": options.lossless,
            "format": options.format.key(),
        }),
    );
    report
}

/// One image; `Ok(None)` when cancelled before its output was kept.
fn export_one(
    source: &Path,
    options: &ExportOptions,
    canceled: &dyn Fn() -> bool,
) -> Result<Option<PathBuf>, String> {
    let format = options.format.resolve(source).map_err(|e| e.to_string())?;
    let extension = format.extension(source);
    let image = decode_oriented(source).map_err(|e| e.to_string())?;
    let size = (image.width(), image.height());
    let target = options
        .fit
        .map(|bounds| fit_within(size, bounds))
        .unwrap_or(size);
    if extension == "webp" && (target.0 > WEBP_MAX_DIMENSION || target.1 > WEBP_MAX_DIMENSION) {
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
    let encoded = options
        .format
        .encode(source, &image, options.lossless, options.quality)
        .map_err(|e| e.to_string())?;
    drop(image);

    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("image")
        .to_string();
    let destination = options.destination.clone();
    let mut pending = Pending::reserve(move |suffix| {
        destination.join(if suffix == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{suffix}.{extension}")
        })
    })
    .map_err(|e| e.to_string())?;
    pending.write(&encoded).map_err(|e| e.to_string())?;
    // Cancelled: dropping the pending file removes it.
    if canceled() {
        return Ok(None);
    }
    let output = pending.publish().map_err(|e| e.to_string())?;
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_crop_and_export_preserve_sixteen_bit_gradient_alpha_and_icc() {
        use image::{ImageDecoder, ImageEncoder};
        let dir = temp_dir("png-depth-profile");
        let source = dir.join("gradient.png");
        let pixels = image::RgbaImage::from_fn(96, 54, |x, y| {
            image::Rgba([x as u8, y as u8, (x + y) as u8, 255])
        });
        let pixels = image::DynamicImage::ImageRgba8(pixels).to_rgba16();
        let pixels = image::ImageBuffer::from_fn(96, 54, |x, y| {
            let mut p = *pixels.get_pixel(x, y);
            p.0[0] += (x % 255) as u16;
            p.0[3] = (x * 500 + y * 100) as u16;
            p
        });
        let icc = include_bytes!("../../tests/assets/srgb.icc").to_vec();
        let mut encoder =
            image::codecs::png::PngEncoder::new(std::fs::File::create(&source).unwrap());
        encoder.set_icc_profile(icc.clone()).unwrap();
        image::DynamicImage::ImageRgba16(pixels.clone())
            .write_with_encoder(encoder)
            .unwrap();
        let before = std::fs::read(&source).unwrap();
        let out = dir.join("out");
        let mut opts = options(&out);
        opts.format = OutputFormat::default();
        opts.lossless = false; // A legacy lossy WebP setting must not affect PNG.
        let report = export_images(std::slice::from_ref(&source), &opts, &|| false, &|_, _| {});
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(report.written, vec![out.join("gradient.png")]);
        let rect = super::super::CropRect {
            x: 7,
            y: 5,
            width: 64,
            height: 36,
        };
        let crop = super::super::crop_copy(&super::super::CropConfig {
            source: source.clone(),
            destination_dir: out.clone(),
            rectangle: rect,
            format: OutputFormat::Png,
        })
        .unwrap()
        .output;
        for (path, expected) in [
            (&report.written[0], pixels.clone()),
            (
                &crop,
                image::imageops::crop_imm(&pixels, 7, 5, 64, 36).to_image(),
            ),
        ] {
            let actual = image::open(path).unwrap();
            assert_eq!(actual.color(), image::ColorType::Rgba16);
            assert_eq!(actual.to_rgba16(), expected);
            let mut decoder = image::ImageReader::open(path)
                .unwrap()
                .into_decoder()
                .unwrap();
            assert_eq!(decoder.icc_profile().unwrap(), Some(icc.clone()));
            assert_eq!(
                decoder.orientation().unwrap(),
                image::metadata::Orientation::NoTransforms
            );
        }
        assert_eq!(std::fs::read(&source).unwrap(), before);
        // Keep this fixture only when explicitly requested for an independent decoder check.
        if let Some(destination) = std::env::var_os("SHARPR_OUTPUT_ORACLE") {
            std::fs::create_dir_all(&destination).unwrap();
            std::fs::copy(&source, Path::new(&destination).join("source.png")).unwrap();
            std::fs::copy(&crop, Path::new(&destination).join("crop.png")).unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn formats_handle_mixed_sources_names_failures_and_original_bytes() {
        let dir = temp_dir("format-matrix");
        let sources: Vec<_> = ["png", "jpg", "webp", "tiff", "bmp"]
            .iter()
            .map(|ext| {
                let path = dir.join(format!("source.{ext}"));
                pattern(64, 40).save(&path).unwrap();
                path
            })
            .collect();
        let before: Vec<_> = sources.iter().map(|p| std::fs::read(p).unwrap()).collect();
        for format in [
            OutputFormat::Png,
            OutputFormat::Webp,
            OutputFormat::Original,
        ] {
            let out = dir.join(format.key());
            let mut opts = options(&out);
            opts.format = format;
            let report = export_images(&sources, &opts, &|| false, &|_, _| {});
            assert!(report.failed.is_empty(), "{:?}", report.failed);
            assert_eq!(report.written.len(), sources.len());
            for (src, output) in sources.iter().zip(&report.written) {
                let extension = if format == OutputFormat::Original {
                    src.extension().unwrap().to_str().unwrap()
                } else {
                    format.key()
                };
                assert_eq!(output.extension().unwrap(), extension);
                if extension != "jpg" {
                    assert_eq!(
                        image::open(output).unwrap().to_rgba8(),
                        image::open(src).unwrap().to_rgba8()
                    );
                }
                let crop = super::super::crop_copy(&super::super::CropConfig {
                    source: src.clone(),
                    destination_dir: out.clone(),
                    format,
                    rectangle: super::super::CropRect {
                        x: 4,
                        y: 3,
                        width: 32,
                        height: 18,
                    },
                })
                .unwrap()
                .output;
                assert_eq!(crop.extension().unwrap(), extension);
                assert_eq!(image::image_dimensions(crop).unwrap(), (32, 18));
            }
            let repeated = export_images(&sources, &opts, &|| false, &|_, _| {});
            assert!(repeated.written.iter().all(|p| !report.written.contains(p)));
        }
        for (source, before) in sources.iter().zip(before) {
            assert_eq!(std::fs::read(source).unwrap(), before);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn png_has_no_webp_limit_and_float_png_fails_without_partial_outputs() {
        let dir = temp_dir("png-limits");
        let wide = dir.join("wide.png");
        pattern(WEBP_MAX_DIMENSION + 1, 1).save(&wide).unwrap();
        let out = dir.join("out");
        let mut opts = options(&out);
        opts.format = OutputFormat::Png;
        let report = export_images(std::slice::from_ref(&wide), &opts, &|| false, &|_, _| {});
        assert_eq!(report.written.len(), 1);
        opts.format = OutputFormat::Webp;
        assert_eq!(
            export_images(&[wide], &opts, &|| false, &|_, _| {})
                .failed
                .len(),
            1
        );
        let source = dir.join("float.tiff");
        image::DynamicImage::ImageRgb32F(image::Rgb32FImage::from_pixel(
            32,
            18,
            image::Rgb([0.123, 1.4, 0.8]),
        ))
        .save(&source)
        .unwrap();
        let before = std::fs::read(&source).unwrap();
        opts.format = OutputFormat::Png;
        let report = export_images(std::slice::from_ref(&source), &opts, &|| false, &|_, _| {});
        assert!(report.written.is_empty());
        assert!(report.failed[0].1.contains("floating-point/HDR"));
        assert_eq!(
            std::fs::read_dir(&out).unwrap().count(),
            1,
            "only earlier wide PNG remains"
        );
        assert_eq!(std::fs::read(source).unwrap(), before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_format_cleans_cancelled_partials_and_original_batches_keep_successes() {
        let dir = temp_dir("format-cancel");
        let source = dir.join("image.png");
        pattern(32, 18).save(&source).unwrap();
        for format in [
            OutputFormat::Png,
            OutputFormat::Webp,
            OutputFormat::Original,
        ] {
            let out = dir.join(format.key());
            let mut opts = options(&out);
            opts.format = format;
            // Allow the loop's initial check, cancel after encoding/writing.
            let checks = std::cell::Cell::new(0);
            let report = export_images(
                std::slice::from_ref(&source),
                &opts,
                &|| {
                    checks.set(checks.get() + 1);
                    checks.get() > 1
                },
                &|_, _| {},
            );
            assert!(report.canceled);
            assert!(report.written.is_empty());
            assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        }
        let unsupported = dir.join("unsupported.ppm");
        std::fs::write(&unsupported, b"P6\n1 1\n255\n\x00\x00\x00").unwrap();
        let mut opts = options(&dir.join("mixed"));
        opts.format = OutputFormat::Original;
        let report = export_images(&[unsupported.clone(), source], &opts, &|| false, &|_, _| {});
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, unsupported);
        assert_eq!(report.written.len(), 1);
        assert_eq!(std::fs::read_dir(&opts.destination).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

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

    fn options(destination: &Path) -> ExportOptions {
        ExportOptions {
            format: OutputFormat::Webp,
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
        let report = export_images(
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
        let report = export_images(&[big.clone(), broken.clone()], &opts, &|| false, &|_, _| {});
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
        let report = export_images(
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
