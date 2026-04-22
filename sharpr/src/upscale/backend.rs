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
