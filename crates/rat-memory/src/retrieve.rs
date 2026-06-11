use std::collections::HashMap;
use std::sync::Arc;

use rat_core::clock::Clock;
use rat_proto::Observation;
use rat_store::rows::{Memory, MemoryFilter};
use rat_store::store::Store;

use crate::embed::{EmbeddingClient, cosine};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitKind {
    Observation,
    Memory,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub id: String,
    pub kind: HitKind,
    pub score: f64,
}

/// Reciprocal Rank Fusion of two ranked lists.
///
/// score(id) = Σ 1/(k + rank_i) over all lists containing `id`.
/// rank is 1-based (position + 1).
/// Returns list sorted descending by score; ties broken by id ascending.
pub fn rrf_fuse(fts: &[String], vec: &[String], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<&str, f64> = HashMap::new();

    for (pos, id) in fts.iter().enumerate() {
        let rank = (pos + 1) as f64;
        *scores.entry(id.as_str()).or_insert(0.0) += 1.0 / (k + rank);
    }
    for (pos, id) in vec.iter().enumerate() {
        let rank = (pos + 1) as f64;
        *scores.entry(id.as_str()).or_insert(0.0) += 1.0 / (k + rank);
    }

    let mut result: Vec<(String, f64)> = scores.into_iter().map(|(id, s)| (id.to_owned(), s)).collect();
    // Sort descending by score; tie-break by id ascending
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
    result
}

/// Recency boost: score × (1 + 0.25 × e^(−age_days/14))
pub fn recency_boost(score: f64, age_days: f64) -> f64 {
    score * (1.0 + 0.25 * (-age_days / 14.0_f64).exp())
}

/// Parameters for a hybrid memory search.
pub struct SearchParams {
    pub query: String,
    pub project_id: Option<String>,
    pub n: usize,
}

/// Hybrid search: FTS + optional vector search, fused with RRF, filtered by project, boosted by recency.
pub async fn search(
    store: &Store,
    embedder: Option<&EmbeddingClient>,
    clock: &Arc<dyn Clock>,
    params: SearchParams,
) -> Result<Vec<Hit>, crate::MemoryError> {
    let now_ms = clock.now_ms();
    let ms_per_day = 86_400_000.0f64;

    // --- FTS ---
    // malformed FTS5 query → empty list, not a hard error
    let fts_obs = store
        .fts_observations(params.query.clone(), 40)
        .await
        .unwrap_or_default();
    // malformed FTS5 query → empty list, not a hard error
    let fts_mem = store
        .fts_memories(params.query.clone(), 40)
        .await
        .unwrap_or_default();

    // --- Vector ---
    let (vec_obs, vec_mem) = if let Some(embedder) = embedder {
        let query_vecs = embedder.embed(std::slice::from_ref(&params.query)).await.map_err(crate::MemoryError::Embed)?;
        let query_vec = &query_vecs[0];

        // Observations
        let all_obs_emb = store.all_observation_embeddings(10_000).await.map_err(crate::MemoryError::Store)?;
        let mut obs_sims: Vec<(String, f32)> = all_obs_emb
            .iter()
            .map(|(id, emb)| (id.clone(), cosine(query_vec, emb)))
            .collect();
        obs_sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let vo: Vec<String> = obs_sims.into_iter().take(40).map(|(id, _)| id).collect();

        // Memories
        let all_mem_emb = store.all_memory_embeddings(10_000).await.map_err(crate::MemoryError::Store)?;
        let mut mem_sims: Vec<(String, f32)> = all_mem_emb
            .iter()
            .map(|(id, emb)| (id.clone(), cosine(query_vec, emb)))
            .collect();
        mem_sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let vm: Vec<String> = mem_sims.into_iter().take(40).map(|(id, _)| id).collect();

        (vo, vm)
    } else {
        (vec![], vec![])
    };

    // --- Fuse observations and memories separately ---
    let fused_obs = rrf_fuse(&fts_obs, &vec_obs, 60.0);
    let fused_mem = rrf_fuse(&fts_mem, &vec_mem, 60.0);

    // --- Build hits ---
    let mut hits: Vec<Hit> = Vec::new();

    // Observations: filter by project_id
    if !fused_obs.is_empty() {
        let obs_ids: Vec<String> = fused_obs.iter().map(|(id, _)| id.clone()).collect();
        let observations = store
            .observations_by_ids(obs_ids)
            .await
            .map_err(crate::MemoryError::Store)?;
        let obs_map: HashMap<String, &Observation> = observations.iter().map(|o| (o.id.clone(), o)).collect();

        for (id, rrf_score) in &fused_obs {
            if let Some(obs) = obs_map.get(id) {
                // Project filter: pass if no project_id filter OR project matches
                let pass = params.project_id.as_ref().is_none_or(|pid| {
                    obs.project_id.as_ref() == Some(pid)
                });
                if !pass {
                    continue;
                }
                let age_days = (now_ms - obs.ts).max(0) as f64 / ms_per_day;
                let boosted = recency_boost(*rrf_score, age_days);
                hits.push(Hit { id: id.clone(), kind: HitKind::Observation, score: boosted });
            }
        }
    }

    // Memories: filter by project and type
    if !fused_mem.is_empty() {
        let memories = store
            .list_memories(MemoryFilter { include_archived: false, ..Default::default() })
            .await
            .map_err(crate::MemoryError::Store)?;
        let mem_map: HashMap<String, &Memory> = memories.iter().map(|m| (m.id.clone(), m)).collect();

        for (id, rrf_score) in &fused_mem {
            if let Some(mem) = mem_map.get(id) {
                // Project filter:
                // - type=="personal" always passes
                // - type=="project" passes only if project_id matches the search's project_id (when given)
                // - other types: pass if project_id matches or search project_id is None
                let pass = if mem.r#type == "personal" {
                    true
                } else if mem.r#type == "project" {
                    params.project_id.as_ref().is_none_or(|pid| {
                        mem.project_id.as_ref() == Some(pid)
                    })
                } else {
                    params.project_id.as_ref().is_none_or(|pid| {
                        mem.project_id.as_ref() == Some(pid)
                    })
                };
                if !pass {
                    continue;
                }
                let age_days = (now_ms - mem.updated).max(0) as f64 / ms_per_day;
                let boosted = recency_boost(*rrf_score, age_days);
                hits.push(Hit { id: id.clone(), kind: HitKind::Memory, score: boosted });
            }
        }
    }

    // Sort all hits descending by score, take n
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
    hits.truncate(params.n);

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use rat_core::clock::FakeClock;
    use rat_store::store::Store;
    use rat_proto::NewObservation;
    use tempfile::tempdir;

    #[test]
    fn rrf_fuse_overlap() {
        // Both lists have "a" and "b"; "c" only in fts
        let fts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vec_list = vec!["b".to_string(), "a".to_string()];
        let result = rrf_fuse(&fts, &vec_list, 60.0);

        // "a": 1/61 + 1/62 = 0.016393 + 0.016129 ≈ 0.032522
        // "b": 1/62 + 1/61 ≈ 0.032522  (same score as a, tie-break by id: a < b)
        // "c": 1/63 ≈ 0.015873
        assert_eq!(result[0].0, "a"); // a before b (same score, a < b)
        assert_eq!(result[1].0, "b");
        assert_eq!(result[2].0, "c");

        // scores should be equal for a and b
        let score_a = result[0].1;
        let score_b = result[1].1;
        assert!((score_a - score_b).abs() < 1e-10, "a and b should have equal scores: {} vs {}", score_a, score_b);
        assert!(result[2].1 < result[0].1);
    }

    #[test]
    fn rrf_fuse_disjoint() {
        let fts = vec!["x".to_string()];
        let vec_list = vec!["y".to_string()];
        // x: 1/61, y: 1/61 — tie-break by id: x < y
        let result = rrf_fuse(&fts, &vec_list, 60.0);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "x");
        assert_eq!(result[1].0, "y");
    }

    #[test]
    fn rrf_fuse_tie_break_deterministic() {
        // Same id in both lists at same position → one entry
        let fts = vec!["z".to_string(), "a".to_string()];
        let vec_list = vec!["z".to_string(), "a".to_string()];
        let result = rrf_fuse(&fts, &vec_list, 60.0);
        assert_eq!(result.len(), 2);
        // z: 2*(1/61) = 0.032786, a: 2*(1/62) = 0.032258
        assert_eq!(result[0].0, "z");
        assert_eq!(result[1].0, "a");
    }

    #[test]
    fn rrf_fuse_empty_lists() {
        let result = rrf_fuse(&[], &[], 60.0);
        assert!(result.is_empty());

        let result = rrf_fuse(&["x".to_string()], &[], 60.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "x");
    }

    #[test]
    fn recency_boost_age_zero() {
        // age=0: score * (1 + 0.25 * e^0) = score * 1.25
        let boosted = recency_boost(1.0, 0.0);
        assert!((boosted - 1.25).abs() < 1e-10, "expected 1.25, got {}", boosted);
    }

    #[test]
    fn recency_boost_age_14() {
        // age=14: score * (1 + 0.25 * e^(-1)) ≈ 1 * (1 + 0.25 * 0.36788) ≈ 1.09197
        let boosted = recency_boost(1.0, 14.0);
        let expected = 1.0 + 0.25 * std::f64::consts::E.recip();
        assert!((boosted - expected).abs() < 1e-6, "expected {}, got {}", expected, boosted);
    }

    #[test]
    fn recency_boost_monotonic() {
        let ages = [0.0, 1.0, 7.0, 14.0, 30.0, 60.0, 180.0];
        let boosts: Vec<f64> = ages.iter().map(|&a| recency_boost(1.0, a)).collect();
        for i in 1..boosts.len() {
            assert!(boosts[i] < boosts[i - 1], "not monotonic at index {}: {} >= {}", i, boosts[i], boosts[i - 1]);
        }
    }

    #[tokio::test]
    async fn search_fts_only_no_embedder() {
        let tmp = tempdir().unwrap();
        let clock: Arc<dyn Clock> = FakeClock::at(86_400_000); // day 1
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        store.add_observation(NewObservation {
            kind: "shell_cmd".into(),
            content: "cargo build --release".into(),
            project_id: Some("proj1".into()),
            ..Default::default()
        }).await.unwrap();

        let hits = search(
            &store,
            None,
            &clock,
            SearchParams { query: "cargo".into(), project_id: Some("proj1".into()), n: 10 },
        ).await.unwrap();

        assert!(!hits.is_empty());
        assert_eq!(hits[0].kind, HitKind::Observation);
    }

    #[tokio::test]
    async fn search_fts_project_filter() {
        let tmp = tempdir().unwrap();
        let clock: Arc<dyn Clock> = FakeClock::at(86_400_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        store.add_observation(NewObservation {
            kind: "shell_cmd".into(),
            content: "cargo test hello world".into(),
            project_id: Some("projA".into()),
            ..Default::default()
        }).await.unwrap();

        // Search with different project — should not find it
        let hits = search(
            &store,
            None,
            &clock,
            SearchParams { query: "cargo".into(), project_id: Some("projB".into()), n: 10 },
        ).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn search_hybrid_with_fake_embeddings() {
        let tmp = tempdir().unwrap();
        let clock: Arc<dyn Clock> = FakeClock::at(86_400_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let obs = store.add_observation(NewObservation {
            kind: "shell_cmd".into(),
            content: "deploy production".into(),
            project_id: Some("p1".into()),
            ..Default::default()
        }).await.unwrap();

        // Store a fake embedding for this observation
        store.set_observation_embedding(obs.id.clone(), vec![1.0f32, 0.0]).await.unwrap();

        // Wiremock returns embedding [1.0, 0.0] for query (perfect match)
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"embedding": [1.0, 0.0]}]
            })))
            .mount(&server)
            .await;

        let embedder = EmbeddingClient::new(server.uri(), "test-key");
        let hits = search(
            &store,
            Some(&embedder),
            &clock,
            SearchParams { query: "deploy".into(), project_id: Some("p1".into()), n: 10 },
        ).await.unwrap();

        assert!(!hits.is_empty());
        let hit = hits.iter().find(|h| h.id == obs.id).expect("should find obs");
        assert_eq!(hit.kind, HitKind::Observation);
    }

    #[tokio::test]
    async fn search_malformed_fts5_query_no_panic() {
        let tmp = tempdir().unwrap();
        let clock: Arc<dyn Clock> = FakeClock::at(86_400_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

        // Add an observation
        store.add_observation(NewObservation {
            kind: "shell_cmd".into(),
            content: "cargo build".into(),
            project_id: Some("p1".into()),
            ..Default::default()
        }).await.unwrap();

        // Search with malformed FTS5 query (unbalanced quotes)
        let result = search(
            &store,
            None,
            &clock,
            SearchParams { query: "\"unbalanced".into(), project_id: Some("p1".into()), n: 10 },
        ).await;

        // Must return Ok (no panic), result may be empty or partial
        assert!(result.is_ok(), "malformed query should degrade gracefully, not panic");
    }
}
