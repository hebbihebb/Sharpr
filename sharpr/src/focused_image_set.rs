use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusedImageSet {
    pub name: String,
    pub generation: u64,
    pub paths: Vec<PathBuf>,
}

pub fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}
