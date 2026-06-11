use std::sync::Arc;

use rat_brain::critic::{MemoryHit, MemorySearcher};
use rat_core::clock::Clock;
use rat_memory::embed::EmbeddingClient;
use rat_memory::retrieve::{search, SearchParams};
use rat_store::store::Store;

pub struct DaemonMemorySearcher {
    pub embedder: Option<EmbeddingClient>,
}

#[async_trait::async_trait]
impl MemorySearcher for DaemonMemorySearcher {
    async fn search(
        &self,
        store: &Store,
        clock: &Arc<dyn Clock>,
        query: String,
        project_id: Option<String>,
        n: usize,
    ) -> Vec<MemoryHit> {
        match search(store, self.embedder.as_ref(), clock, SearchParams { query, project_id, n })
            .await
        {
            Ok(hits) => hits.into_iter().map(|h| MemoryHit { id: h.id }).collect(),
            Err(e) => {
                tracing::warn!("memory search error: {e}");
                vec![]
            }
        }
    }
}
