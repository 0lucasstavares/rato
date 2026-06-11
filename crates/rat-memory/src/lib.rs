pub mod embed;
pub mod retrieve;
pub mod jobs;

pub use embed::EmbeddingClient;
pub use retrieve::{Hit, HitKind, rrf_fuse, recency_boost, search};
pub use jobs::{hourly, nightly};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("store: {0}")]
    Store(#[from] rat_store::error::StoreError),
    #[error("embed: {0}")]
    Embed(#[from] crate::embed::EmbedError),
    #[error("llm: {0}")]
    Llm(#[from] rat_brain::error::LlmError),
}
