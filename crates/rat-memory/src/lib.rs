pub mod embed;
pub mod jobs;
pub mod retrieve;

pub use embed::EmbeddingClient;
pub use jobs::{hourly, nightly};
pub use retrieve::{recency_boost, rrf_fuse, search, Hit, HitKind};

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
