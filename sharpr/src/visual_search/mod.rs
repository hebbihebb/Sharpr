//! Visual search: find images by describing them (`ROADMAP.md` Phase 5,
//! decisions D22–D24).
//!
//! EmbeddingGemma 2 turns images and search words into vectors in the same
//! space; a search ranks images by how close their vectors are to the
//! words'. The model runs in `llama-server` child processes that Sharpr
//! starts on demand and talks to only over loopback (`runtime`). The user
//! installs the model files once into a folder Sharpr checks (`files`);
//! nothing is downloaded. Vectors live in their own SQLite file, keyed by
//! content fingerprint (`store`), so deleting it loses nothing but time.
//! Everything here is GTK-free.

pub mod files;
pub mod index;
pub mod prepare;
pub mod runtime;
pub mod search;
pub mod similar;
pub mod store;

/// Length of every EmbeddingGemma 2 vector.
pub const DIMENSIONS: usize = 768;

/// Bumped whenever image preparation changes, so vectors made the old way
/// are no longer used.
pub const PREPARATION_VERSION: u32 = 1;

/// What EmbeddingGemma expects in front of a search query; the Phase 3
/// benchmark used the same prompt.
pub const QUERY_PROMPT: &str = "task: search result | query: ";

/// Cosine similarity of two unit vectors.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
