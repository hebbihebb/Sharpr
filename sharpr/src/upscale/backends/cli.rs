use std::path::PathBuf;

use crate::upscale::{
    backend::UpscaleBackend,
    runner::{UpscaleEvent, UpscaleRunner},
    UpscaleJobConfig,
};

pub struct CliBackend {
    binary: PathBuf,
}

impl CliBackend {
    pub fn new(binary: PathBuf) -> Self {
        Self { binary }
    }
}

impl UpscaleBackend for CliBackend {
    fn run(
        self: Box<Self>,
        input: PathBuf,
        output: PathBuf,
        config: UpscaleJobConfig,
    ) -> async_channel::Receiver<UpscaleEvent> {
        UpscaleRunner::run(&self.binary, &input, &output, config)
    }
}
