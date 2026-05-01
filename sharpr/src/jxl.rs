use std::mem::MaybeUninit;
use std::path::Path;
use std::ptr;
use std::time::Instant;

use image::DynamicImage;
use jpegxl_rs::encode::{EncoderFrame, EncoderResult, EncoderSpeed};
use jpegxl_rs::image::ToDynamic;
use jpegxl_rs::parallel::threads_runner::ThreadsRunner;
use jpegxl_rs::{decoder_builder, encoder_builder};
use jpegxl_sys::common::types::{JxlBool, JxlDataType, JxlEndianness, JxlPixelFormat};
use jpegxl_sys::decode::{
    JxlDecoderCloseInput, JxlDecoderCreate, JxlDecoderDestroy, JxlDecoderGetBasicInfo,
    JxlDecoderPreviewOutBufferSize, JxlDecoderProcessInput, JxlDecoderSetInput,
    JxlDecoderSetPreviewOutBuffer, JxlDecoderStatus, JxlDecoderSubscribeEvents,
};
use jpegxl_sys::metadata::codestream_header::JxlBasicInfo;

const DEFAULT_DECODE_WORKERS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedPreviewInfo {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DecodedEmbeddedPreview {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn is_jxl_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("jxl"))
        .unwrap_or(false)
}

pub fn decode_path(path: &Path) -> Result<DynamicImage, String> {
    decode_path_with_num_workers(path, DEFAULT_DECODE_WORKERS)
}

fn decode_path_with_num_workers(path: &Path, num_workers: usize) -> Result<DynamicImage, String> {
    let data = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let parallel_runner = ThreadsRunner::new(None, Some(num_workers.max(1)))
        .ok_or_else(|| "create JPEG XL thread pool".to_string())?;
    let decoder = decoder_builder()
        .parallel_runner(&parallel_runner)
        .build()
        .map_err(|err| format!("create JPEG XL decoder: {err}"))?;
    decoder
        .decode_to_image(&data)
        .map_err(|err| format!("decode JPEG XL {}: {err}", path.display()))?
        .ok_or_else(|| format!("decode JPEG XL {}: no image data returned", path.display()))
}

pub fn image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let data = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let (metadata, _) = decoder_builder()
        .build()
        .map_err(|err| format!("create JPEG XL decoder: {err}"))?
        .decode(&data)
        .map_err(|err| format!("decode JPEG XL metadata {}: {err}", path.display()))?;
    Ok((metadata.width, metadata.height))
}

pub fn preview_info(path: &Path) -> Result<Option<EmbeddedPreviewInfo>, String> {
    let data = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let Some(info) = decode_basic_info(&data)? else {
        return Ok(None);
    };
    Ok(preview_info_from_basic_info(&info))
}

pub fn decode_embedded_preview(path: &Path) -> Result<Option<DecodedEmbeddedPreview>, String> {
    let data = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    decode_embedded_preview_from_bytes(&data, path)
}

pub fn encode_path(
    image: &DynamicImage,
    output: &Path,
    quality: u8,
    lossless: bool,
    effort: u8,
) -> Result<(), String> {
    let started = Instant::now();
    let quality = quality.clamp(0, 100) as f32;
    let parallel_runner =
        ThreadsRunner::new(None, None).ok_or_else(|| "create JPEG XL thread pool".to_string())?;
    let mut encoder = encoder_builder()
        .has_alpha(image.color().has_alpha())
        .lossless(lossless)
        .speed(speed_for_effort(effort))
        .jpeg_quality(if lossless { 100.0 } else { quality })
        .parallel_runner(&parallel_runner)
        .build()
        .map_err(|err| format!("create JPEG XL encoder: {err}"))?;
    crate::bench_event!(
        "jxl.encode.start",
        serde_json::json!({
            "output": output.display().to_string(),
            "width": image.width(),
            "height": image.height(),
            "has_alpha": image.color().has_alpha(),
            "quality": quality,
            "lossless": lossless,
            "effort": effort,
        }),
    );

    let encoded: EncoderResult<u8> = if image.color().has_alpha() {
        let rgba_started = Instant::now();
        let rgba = image.to_rgba8();
        let rgba_ms = crate::bench::duration_ms(rgba_started);
        let encode_started = Instant::now();
        let result = encoder
            .encode_frame(
                &EncoderFrame::new(rgba.as_raw()).num_channels(4),
                rgba.width(),
                rgba.height(),
            )
            .map_err(|err| format!("encode JPEG XL {}: {err}", output.display()))?;
        crate::bench_event!(
            "jxl.encode.stage",
            serde_json::json!({
                "output": output.display().to_string(),
                "pixel_prep_ms": rgba_ms,
                "encode_ms": crate::bench::duration_ms(encode_started),
                "path": "rgba",
            }),
        );
        result
    } else {
        let rgb_started = Instant::now();
        let rgb = image.to_rgb8();
        let rgb_ms = crate::bench::duration_ms(rgb_started);
        let encode_started = Instant::now();
        let result = encoder
            .encode(rgb.as_raw(), rgb.width(), rgb.height())
            .map_err(|err| format!("encode JPEG XL {}: {err}", output.display()))?;
        crate::bench_event!(
            "jxl.encode.stage",
            serde_json::json!({
                "output": output.display().to_string(),
                "pixel_prep_ms": rgb_ms,
                "encode_ms": crate::bench::duration_ms(encode_started),
                "path": "rgb",
            }),
        );
        result
    };

    let write_started = Instant::now();
    std::fs::write(output, encoded.data)
        .map_err(|err| format!("write JPEG XL {}: {err}", output.display()))?;
    crate::bench_event!(
        "jxl.encode.done",
        serde_json::json!({
            "output": output.display().to_string(),
            "write_ms": crate::bench::duration_ms(write_started),
            "duration_ms": crate::bench::duration_ms(started),
        }),
    );
    Ok(())
}

fn speed_for_effort(effort: u8) -> EncoderSpeed {
    match effort.clamp(1, 10) {
        1 => EncoderSpeed::Lightning,
        2 => EncoderSpeed::Thunder,
        3 => EncoderSpeed::Falcon,
        4 => EncoderSpeed::Cheetah,
        5 => EncoderSpeed::Hare,
        6 => EncoderSpeed::Wombat,
        7 => EncoderSpeed::Squirrel,
        8 => EncoderSpeed::Kitten,
        9 => EncoderSpeed::Tortoise,
        _ => EncoderSpeed::Glacier,
    }
}

fn decode_basic_info(data: &[u8]) -> Result<Option<JxlBasicInfo>, String> {
    let decoder = DecoderHandle::new()?;
    decoder.subscribe_events(JxlDecoderStatus::BasicInfo as i32)?;
    decoder.set_input(data)?;

    let mut basic_info = MaybeUninit::<JxlBasicInfo>::uninit();
    loop {
        match decoder.process_input() {
            JxlDecoderStatus::BasicInfo => {
                decoder.get_basic_info(&mut basic_info)?;
                return Ok(Some(unsafe { basic_info.assume_init() }));
            }
            JxlDecoderStatus::NeedMoreInput | JxlDecoderStatus::Success => return Ok(None),
            JxlDecoderStatus::Error => {
                return Err("decode JPEG XL basic info: decoder error".into())
            }
            status => {
                return Err(format!(
                    "decode JPEG XL basic info: unexpected decoder status {status:?}"
                ))
            }
        }
    }
}

fn decode_embedded_preview_from_bytes(
    data: &[u8],
    path: &Path,
) -> Result<Option<DecodedEmbeddedPreview>, String> {
    let decoder = DecoderHandle::new()?;
    decoder.subscribe_events(
        JxlDecoderStatus::BasicInfo as i32 | JxlDecoderStatus::PreviewImage as i32,
    )?;
    decoder.set_input(data)?;

    let pixel_format = JxlPixelFormat {
        num_channels: 4,
        data_type: JxlDataType::Uint8,
        endianness: JxlEndianness::Native,
        align: 0,
    };

    let mut basic_info = MaybeUninit::<JxlBasicInfo>::uninit();
    let mut preview_info = None;
    let mut rgba = Vec::new();

    loop {
        match decoder.process_input() {
            JxlDecoderStatus::BasicInfo => {
                decoder.get_basic_info(&mut basic_info)?;
                preview_info =
                    preview_info_from_basic_info(unsafe { basic_info.assume_init_ref() });
                if preview_info.is_none() {
                    return Ok(None);
                }
            }
            JxlDecoderStatus::NeedPreviewOutBuffer => {
                let Some(info) = preview_info else {
                    return Err(format!(
                        "decode JPEG XL preview {}: preview buffer requested without preview header",
                        path.display()
                    ));
                };
                let size = decoder.preview_out_buffer_size(&pixel_format)?;
                let expected = usize::try_from(info.width)
                    .ok()
                    .and_then(|width| {
                        usize::try_from(info.height)
                            .ok()
                            .map(|height| (width, height))
                    })
                    .and_then(|(width, height)| width.checked_mul(height))
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or_else(|| {
                        format!(
                            "decode JPEG XL preview {}: invalid preview dimensions {}x{}",
                            path.display(),
                            info.width,
                            info.height
                        )
                    })?;
                if size != expected {
                    return Err(format!(
                        "decode JPEG XL preview {}: preview buffer size mismatch ({size} != {expected})",
                        path.display()
                    ));
                }
                rgba.resize(size, 0);
                decoder.set_preview_out_buffer(&pixel_format, &mut rgba)?;
            }
            JxlDecoderStatus::PreviewImage => {
                let Some(info) = preview_info else {
                    return Err(format!(
                        "decode JPEG XL preview {}: preview decoded without preview header",
                        path.display()
                    ));
                };
                return Ok(Some(DecodedEmbeddedPreview {
                    rgba,
                    width: info.width,
                    height: info.height,
                }));
            }
            JxlDecoderStatus::NeedMoreInput | JxlDecoderStatus::Success => return Ok(None),
            JxlDecoderStatus::Error => {
                return Err(format!(
                    "decode JPEG XL preview {}: decoder error",
                    path.display()
                ))
            }
            status => {
                return Err(format!(
                    "decode JPEG XL preview {}: unexpected decoder status {status:?}",
                    path.display()
                ))
            }
        }
    }
}

fn preview_info_from_basic_info(info: &JxlBasicInfo) -> Option<EmbeddedPreviewInfo> {
    if info.have_preview == JxlBool::False {
        return None;
    }

    Some(EmbeddedPreviewInfo {
        width: info.preview.xsize,
        height: info.preview.ysize,
    })
}

fn check_decoder_status(status: JxlDecoderStatus, context: &str) -> Result<(), String> {
    match status {
        JxlDecoderStatus::Success => Ok(()),
        JxlDecoderStatus::Error => Err(format!("{context}: decoder error")),
        other => Err(format!("{context}: unexpected decoder status {other:?}")),
    }
}

struct DecoderHandle(*mut jpegxl_sys::decode::JxlDecoder);

impl DecoderHandle {
    fn new() -> Result<Self, String> {
        let decoder = unsafe { JxlDecoderCreate(ptr::null()) };
        if decoder.is_null() {
            return Err("create JPEG XL decoder".into());
        }
        Ok(Self(decoder))
    }

    fn subscribe_events(&self, events: i32) -> Result<(), String> {
        check_decoder_status(
            unsafe { JxlDecoderSubscribeEvents(self.0, events) },
            "subscribe JPEG XL decoder events",
        )
    }

    fn set_input(&self, data: &[u8]) -> Result<(), String> {
        check_decoder_status(
            unsafe { JxlDecoderSetInput(self.0, data.as_ptr(), data.len()) },
            "set JPEG XL decoder input",
        )?;
        unsafe { JxlDecoderCloseInput(self.0) };
        Ok(())
    }

    fn process_input(&self) -> JxlDecoderStatus {
        unsafe { JxlDecoderProcessInput(self.0) }
    }

    fn get_basic_info(&self, info: &mut MaybeUninit<JxlBasicInfo>) -> Result<(), String> {
        check_decoder_status(
            unsafe { JxlDecoderGetBasicInfo(self.0, info.as_mut_ptr()) },
            "read JPEG XL basic info",
        )
    }

    fn preview_out_buffer_size(&self, format: &JxlPixelFormat) -> Result<usize, String> {
        let mut size = 0usize;
        check_decoder_status(
            unsafe { JxlDecoderPreviewOutBufferSize(self.0, format, &mut size) },
            "read JPEG XL preview buffer size",
        )?;
        Ok(size)
    }

    fn set_preview_out_buffer(
        &self,
        format: &JxlPixelFormat,
        buffer: &mut [u8],
    ) -> Result<(), String> {
        check_decoder_status(
            unsafe {
                JxlDecoderSetPreviewOutBuffer(
                    self.0,
                    format,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            },
            "set JPEG XL preview output buffer",
        )
    }
}

impl Drop for DecoderHandle {
    fn drop(&mut self) {
        unsafe { JxlDecoderDestroy(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sharpr-jxl-{name}-{}-{nanos}.jxl",
            std::process::id()
        ))
    }

    #[test]
    fn preview_helpers_return_none_when_preview_is_absent() {
        let path = temp_path("no-preview");
        let mut rgba = RgbaImage::new(8, 6);
        rgba.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        rgba.put_pixel(7, 5, Rgba([0, 0, 255, 255]));
        let image = DynamicImage::ImageRgba8(rgba);

        encode_path(&image, &path, 90, false, 7).unwrap();

        assert_eq!(preview_info(&path).unwrap(), None);
        assert_eq!(decode_embedded_preview(&path).unwrap(), None);

        let _ = std::fs::remove_file(path);
    }
}
