use std::path::{Path, PathBuf};

use ndarray::Array4;
use ort::session::Session;
use ort::value::Tensor;

use crate::upscale::{
    backend::UpscaleBackend,
    runner::{save_image, UpscaleEvent, UpscaleRunner},
    tiling::{process_tiled, TileConfig},
    OnnxUpscaleModel, UpscaleJobConfig,
};

pub struct OnnxBackend {
    model: OnnxUpscaleModel,
}

impl OnnxBackend {
    pub fn new(model: OnnxUpscaleModel) -> Self {
        Self { model }
    }

    pub fn model_path(model: OnnxUpscaleModel) -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sharpr")
            .join("models")
            .join(model.filename())
    }
}

impl UpscaleBackend for OnnxBackend {
    fn run(
        self: Box<Self>,
        input: PathBuf,
        output: PathBuf,
        config: UpscaleJobConfig,
    ) -> async_channel::Receiver<UpscaleEvent> {
        let (tx, rx) = async_channel::bounded(64);
        let model = self.model;
        std::thread::spawn(move || run_onnx_job(input, output, config, model, tx));
        rx
    }
}

fn run_onnx_job(
    input: PathBuf,
    output: PathBuf,
    config: UpscaleJobConfig,
    model: OnnxUpscaleModel,
    tx: async_channel::Sender<UpscaleEvent>,
) {
    let send = |ev: UpscaleEvent| {
        let _ = tx.send_blocking(ev);
    };

    let model_path = OnnxBackend::model_path(model);
    if !model_path.exists() {
        send(UpscaleEvent::Failed(format!(
            "ONNX model not found: {}\nPlace {} there to use the ONNX backend.",
            model_path.display(),
            model.filename()
        )));
        return;
    }

    send(UpscaleEvent::Progress(None));

    let img = match image::open(&input) {
        Ok(img) => img.to_rgb8(),
        Err(e) => {
            send(UpscaleEvent::Failed(format!("Failed to open input: {e}")));
            return;
        }
    };

    // Load ORT session once; all tiles reuse it.
    let mut session = match load_session(&model_path) {
        Ok(s) => s,
        Err(e) => {
            send(UpscaleEvent::Failed(e));
            return;
        }
    };

    let input_name = session.inputs()[0].name().to_owned();

    let model_info = model.info();
    let tile_config = TileConfig {
        tile_size: config.tile_size.unwrap_or(256) as usize,
        overlap: 16,
        scale: model_info.native_scale,
    };

    let upscaled = {
        let input_name = &input_name;
        let send_ref = &send;
        process_tiled(
            &img,
            &tile_config,
            |fraction| send_ref(UpscaleEvent::Progress(Some(fraction * 0.9))),
            |tile| run_tile(&mut session, &tile, model, input_name),
        )
    };

    let upscaled = match upscaled {
        Ok(img) => img,
        Err(e) => {
            send(UpscaleEvent::Failed(e));
            return;
        }
    };

    send(UpscaleEvent::Progress(Some(0.95)));

    let (input_w, input_h) = config.source_dimensions;
    let target_w = input_w.saturating_mul(config.requested_scale);
    let target_h = input_h.saturating_mul(config.requested_scale);

    let final_image = if upscaled.width() == target_w && upscaled.height() == target_h {
        image::DynamicImage::ImageRgb8(upscaled)
    } else {
        use image::imageops::FilterType;
        image::DynamicImage::ImageRgb8(upscaled).resize_exact(
            target_w,
            target_h,
            FilterType::Lanczos3,
        )
    };

    let target_format =
        UpscaleRunner::select_output_format(&input, config.output_format, config.compression_mode);

    match save_image(
        final_image,
        &output,
        target_format,
        config.compression_mode,
        config.quality,
    ) {
        Ok(()) => send(UpscaleEvent::Done(output)),
        Err(e) => send(UpscaleEvent::Failed(e)),
    }
}

fn load_session(model_path: &Path) -> Result<Session, String> {
    Session::builder()
        .map_err(|e| format!("ORT init failed: {e}"))?
        .commit_from_file(model_path)
        .map_err(|e| format!("Failed to load {}: {e}", model_path.display()))
}

/// Run a single tile through the ONNX model.
///
/// Pads the tile to the next multiple of `window_size` before inference, then
/// crops the output back to `4 × original tile dimensions`.
fn run_tile(
    session: &mut Session,
    tile: &image::RgbImage,
    model: OnnxUpscaleModel,
    input_name: &str,
) -> Result<image::RgbImage, String> {
    let model_info = model.info();
    let orig_w = tile.width() as usize;
    let orig_h = tile.height() as usize;
    let window = model_info.window_size;
    let scale = model_info.native_scale;

    let pad_h = (window - orig_h % window) % window;
    let pad_w = (window - orig_w % window) % window;
    let ph = orig_h + pad_h;
    let pw = orig_w + pad_w;

    let raw = tile.as_raw();
    let mut tensor = Array4::<f32>::zeros((1, 3, ph, pw));
    for y in 0..orig_h {
        for x in 0..orig_w {
            let base = (y * orig_w + x) * 3;
            tensor[[0, 0, y, x]] = raw[base] as f32 / 255.0;
            tensor[[0, 1, y, x]] = raw[base + 1] as f32 / 255.0;
            tensor[[0, 2, y, x]] = raw[base + 2] as f32 / 255.0;
        }
    }

    let ort_tensor = Tensor::<f32>::from_array(tensor)
        .map_err(|e| format!("Failed to create ORT input tensor: {e}"))?;

    let outputs = session
        .run(ort::inputs![input_name => ort_tensor])
        .map_err(|e| format!("ONNX inference failed: {e}"))?;

    let output_owned = outputs[0usize]
        .try_extract_array::<f32>()
        .map_err(|e| format!("Failed to extract ORT output: {e}"))?
        .to_owned();
    drop(outputs);

    let shape = output_owned.shape().to_vec();
    let crop_h = orig_h * scale;
    let crop_w = orig_w * scale;
    if shape.len() < 4 || shape[2] < crop_h || shape[3] < crop_w {
        return Err(format!(
            "Output shape {shape:?} smaller than expected {crop_w}×{crop_h}"
        ));
    }

    let mut result = image::RgbImage::new(crop_w as u32, crop_h as u32);
    for y in 0..crop_h {
        for x in 0..crop_w {
            let r = (output_owned[[0, 0, y, x]] * 255.0).clamp(0.0, 255.0) as u8;
            let g = (output_owned[[0, 1, y, x]] * 255.0).clamp(0.0, 255.0) as u8;
            let b = (output_owned[[0, 2, y, x]] * 255.0).clamp(0.0, 255.0) as u8;
            result.put_pixel(x as u32, y as u32, image::Rgb([r, g, b]));
        }
    }

    Ok(result)
}
