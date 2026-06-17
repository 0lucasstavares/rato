use std::sync::Arc;

use rat_brain::critic::{MemoryHit, MemorySearcher};
use rat_core::clock::Clock;
use rat_memory::embed::EmbeddingClient;
use rat_memory::retrieve::{search, SearchParams};
use rat_store::store::Store;

pub struct DaemonMemorySearcher {
    pub embedder: Option<EmbeddingClient>,
    pub llm_status: std::sync::Arc<crate::server::LlmStatusState>,
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
        let embedder = self.embedder.as_ref().filter(|_| {
            self.llm_status
                .embedding_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
        });
        match search(
            store,
            embedder,
            clock,
            SearchParams {
                query,
                project_id,
                n,
            },
        )
        .await
        {
            Ok(hits) => hits.into_iter().map(|h| MemoryHit { id: h.id }).collect(),
            Err(e) => {
                let msg = e.to_string();
                // a 4xx from the embeddings API is permanent (key/account/model
                // restriction) — degrade to FTS-only instead of erroring forever
                if msg.contains("HTTP 4") {
                    self.llm_status
                        .embedding_enabled
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                    if let Ok(mut le) = self.llm_status.last_error.lock() {
                        *le = Some(format!("embedding disabled: {msg}"));
                    }
                    tracing::warn!("embedding disabled after 4xx: {msg}");
                } else {
                    tracing::warn!("memory search error: {msg}");
                }
                vec![]
            }
        }
    }
}
