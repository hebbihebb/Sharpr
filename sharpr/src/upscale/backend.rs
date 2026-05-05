use std::path::PathBuf;

use crate::upscale::{runner::UpscaleEvent, UpscaleJobConfig};

/// Abstraction over different upscale implementations.
///
/// Each backend is consumed on `run()` — construct a fresh one per job.
pub trait UpscaleBackend: Send + 'static {
    fn run(
        self: Box<Self>,
        input: PathBuf,
        output: PathBuf,
        config: UpscaleJobConfig,
    ) -> async_channel::Receiver<UpscaleEvent>;
}

pub fn make_upscale_backend(
    backend_kind: crate::upscale::UpscaleBackendKind,
    binary: Option<PathBuf>,
    onnx_model: crate::upscale::OnnxUpscaleModel,
    comfyui_url: &str,
    comfyui_workflow: crate::upscale::ComfyUiWorkflow,
) -> Option<Box<dyn UpscaleBackend>> {
    use crate::upscale::backends::cli::CliBackend;
    use crate::upscale::backends::onnx::OnnxBackend;
    use crate::upscale::ComfyUiBackend;

    match backend_kind {
        crate::upscale::UpscaleBackendKind::Cli => {
            binary.map(|bin| Box::new(CliBackend::new(bin)) as Box<dyn UpscaleBackend>)
        }
        crate::upscale::UpscaleBackendKind::Onnx => {
            Some(Box::new(OnnxBackend::new(onnx_model)) as Box<dyn UpscaleBackend>)
        }
        crate::upscale::UpscaleBackendKind::ComfyUi => {
            Some(Box::new(ComfyUiBackend::new(comfyui_url, comfyui_workflow))
                as Box<dyn UpscaleBackend>)
        }
    }
}
