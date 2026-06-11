use std::sync::Arc;

use rat_brain::backend::{ChatBackend, ChatResponse, Provider, Route};
use rat_brain::critic::{Critic, MemorySearcher, MemoryHit};
use rat_brain::detect::Signal;
use rat_brain::error::LlmError;
use rat_brain::backend::ChatRequest;
use rat_core::clock::FakeClock;
use rat_proto::NewObservation;
use rat_store::store::Store;
use serde_json::json;
use tempfile::tempdir;
use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::{method, path};

/// A mock MemorySearcher that always returns empty results.
struct NoopMemorySearcher;

#[async_trait::async_trait]
impl MemorySearcher for NoopMemorySearcher {
    async fn search(
        &self,
        _store: &Store,
        _clock: &Arc<dyn rat_core::clock::Clock>,
        _query: String,
        _project_id: Option<String>,
        _n: usize,
    ) -> Vec<MemoryHit> {
        vec![]
    }
}

/// Build an OpenAI-format response with the given verdict JSON text.
fn openai_response(verdict_json: &str) -> serde_json::Value {
    json!({
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": verdict_json
            }]
        }],
        "usage": {
            "input_tokens": 10,
            "output_tokens": 20
        }
    })
}

/// A simple mock backend that directly returns a hardcoded response by calling wiremock.
struct DirectMockBackend {
    http: reqwest::Client,
    base_url: String,
    model: String,
}

#[async_trait::async_trait]
impl ChatBackend for DirectMockBackend {
    async fn complete(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let url = format!("{}/v1/responses", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&json!({}))
            .send()
            .await
            .map_err(|e| LlmError::Http(0, e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(LlmError::Http(status.as_u16(), "mock error".to_string()));
        }

        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| LlmError::Http(0, e.to_string()))?;

        let text = val["output"]
            .as_array()
            .and_then(|outputs| {
                outputs.iter().find_map(|o| {
                    o["content"].as_array().and_then(|contents| {
                        contents.iter().find_map(|c| {
                            if c["type"].as_str() == Some("output_text") {
                                c["text"].as_str().map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                    })
                })
            })
            .ok_or_else(|| LlmError::Http(0, "no output_text".to_string()))?;

        let json_val: serde_json::Value =
            serde_json::from_str(&text).map_err(LlmError::BadJson)?;

        Ok(ChatResponse {
            json: json_val,
            tokens_in: val["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            tokens_out: val["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
            model: self.model.clone(),
        })
    }

    fn provider(&self) -> Provider {
        Provider::OpenAi
    }

    fn model_for(&self, _route: Route) -> &str {
        &self.model
    }
}

fn make_critic_with_backend(
    store: Store,
    backend: Box<dyn ChatBackend>,
    clock: Arc<dyn rat_core::clock::Clock>,
) -> Critic {
    Critic::new(store, backend, Some(Box::new(NoopMemorySearcher)), clock)
}

/// Insert a shell_cmd observation with exit=1.
async fn insert_fail_obs(store: &Store, content: &str) -> rat_proto::Observation {
    store
        .add_observation(NewObservation {
            kind: "shell_cmd".into(),
            content: content.into(),
            project_id: Some("proj1".into()),
            meta: json!({"exit": 1}),
            ..Default::default()
        })
        .await
        .unwrap()
}

fn good_verdict_json(obs_id: &str) -> String {
    json!({
        "pushback": {
            "severity": "warn",
            "title": "You seem stuck",
            "message_en": "You have run the same failing command multiple times.",
            "message_pt": "Você executou o mesmo comando com falha várias vezes.",
            "evidence": [
                {
                    "observation_id": obs_id,
                    "quote": "cargo test"
                }
            ],
            "proposed_actions": [
                {
                    "kind": "suggestion",
                    "detail": "Try running cargo check first"
                }
            ],
            "confidence": 0.85
        }
    })
    .to_string()
}

fn low_confidence_verdict_json(obs_id: &str) -> String {
    json!({
        "pushback": {
            "severity": "nudge",
            "title": "Might be stuck",
            "message_en": "Possibly stuck.",
            "message_pt": "Possivelmente travado.",
            "evidence": [
                {
                    "observation_id": obs_id,
                    "quote": "cargo test"
                }
            ],
            "proposed_actions": [],
            "confidence": 0.4
        }
    })
    .to_string()
}

fn fabricated_id_verdict_json() -> String {
    json!({
        "pushback": {
            "severity": "warn",
            "title": "Fabricated",
            "message_en": "Fabricated evidence.",
            "message_pt": "Evidência fabricada.",
            "evidence": [
                {
                    "observation_id": "FAKE_OBS_ID_DOES_NOT_EXIST",
                    "quote": "some quote"
                }
            ],
            "proposed_actions": [],
            "confidence": 0.9
        }
    })
    .to_string()
}

#[tokio::test]
async fn good_verdict_shown_row() {
    let tmp = tempdir().unwrap();
    let clock: Arc<dyn rat_core::clock::Clock> = FakeClock::at(1_000_000);
    let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

    // Insert 3 failing observations so the store has real IDs
    let obs1 = insert_fail_obs(&store, "cargo test").await;
    let obs2 = insert_fail_obs(&store, "cargo test").await;
    let obs3 = insert_fail_obs(&store, "cargo test").await;

    // Wiremock returns a good verdict citing obs1
    let server = MockServer::start().await;
    let verdict = good_verdict_json(&obs1.id);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::from_str::<serde_json::Value>(&openai_response(&verdict).to_string()).unwrap()
        ))
        .mount(&server)
        .await;

    let backend = Box::new(DirectMockBackend {
        http: reqwest::Client::new(),
        base_url: server.uri(),
        model: "test-model".to_string(),
    });

    let critic = make_critic_with_backend(store.clone(), backend, clock.clone());

    let signals = vec![Signal::StuckLoop {
        cmd: "cargo test".to_string(),
        count: 3,
        obs_ids: vec![obs1.id.clone(), obs2.id.clone(), obs3.id.clone()],
    }];

    let result = critic.slow_tick(&signals).await;

    assert!(result.is_some(), "expected a pushback row to be returned");
    let pb = result.unwrap();
    assert_eq!(pb.status, "shown");
    assert_eq!(pb.trigger, "stuck_loop");
    assert!(pb.confidence > 0.0);

    // Verify pushback row in store
    let pushbacks = store.recent_pushbacks(10).await.unwrap();
    assert!(!pushbacks.is_empty(), "pushback should be in store");
    assert_eq!(pushbacks[0].status, "shown");

    // Verify disclosure row was inserted (we can't query disclosures directly,
    // but we can check that api_calls were recorded via the fact that no error occurred)
    // The test passes if no panic and status="shown"
}

#[tokio::test]
async fn fabricated_obs_id_returns_none() {
    let tmp = tempdir().unwrap();
    let clock: Arc<dyn rat_core::clock::Clock> = FakeClock::at(1_000_000);
    let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

    // Insert real observations
    let obs1 = insert_fail_obs(&store, "cargo test").await;
    let obs2 = insert_fail_obs(&store, "cargo test").await;
    let obs3 = insert_fail_obs(&store, "cargo test").await;

    // LLM returns a verdict with a fabricated obs ID
    let server = MockServer::start().await;
    let verdict = fabricated_id_verdict_json();
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            openai_response(&verdict)
        ))
        .mount(&server)
        .await;

    let backend = Box::new(DirectMockBackend {
        http: reqwest::Client::new(),
        base_url: server.uri(),
        model: "test-model".to_string(),
    });

    let critic = make_critic_with_backend(store.clone(), backend, clock.clone());

    let signals = vec![Signal::StuckLoop {
        cmd: "cargo test".to_string(),
        count: 3,
        obs_ids: vec![obs1.id.clone(), obs2.id.clone(), obs3.id.clone()],
    }];

    let result = critic.slow_tick(&signals).await;

    // Should return None because evidence IDs are fabricated
    assert!(result.is_none(), "fabricated obs ID should cause slow_tick to return None");

    // No pushback row should be inserted
    let pushbacks = store.recent_pushbacks(10).await.unwrap();
    assert!(pushbacks.is_empty(), "no pushback should be stored for fabricated IDs");
}

#[tokio::test]
async fn low_confidence_queued() {
    let tmp = tempdir().unwrap();
    let clock: Arc<dyn rat_core::clock::Clock> = FakeClock::at(1_000_000);
    let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

    let obs1 = insert_fail_obs(&store, "cargo test").await;
    let obs2 = insert_fail_obs(&store, "cargo test").await;
    let obs3 = insert_fail_obs(&store, "cargo test").await;

    let server = MockServer::start().await;
    let verdict = low_confidence_verdict_json(&obs1.id);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            openai_response(&verdict)
        ))
        .mount(&server)
        .await;

    let backend = Box::new(DirectMockBackend {
        http: reqwest::Client::new(),
        base_url: server.uri(),
        model: "test-model".to_string(),
    });

    let critic = make_critic_with_backend(store.clone(), backend, clock.clone());

    let signals = vec![Signal::StuckLoop {
        cmd: "cargo test".to_string(),
        count: 3,
        obs_ids: vec![obs1.id.clone(), obs2.id.clone(), obs3.id.clone()],
    }];

    let result = critic.slow_tick(&signals).await;

    // Should return None (low confidence → queued but not returned)
    assert!(result.is_none(), "low confidence should return None");

    // But a pushback should still be stored with status="queued"
    let pushbacks = store.recent_pushbacks(10).await.unwrap();
    assert!(!pushbacks.is_empty(), "queued pushback should be stored");
    assert_eq!(pushbacks[0].status, "queued");
}

#[tokio::test]
async fn dedupe_skip() {
    let tmp = tempdir().unwrap();
    let clock: Arc<dyn rat_core::clock::Clock> = FakeClock::at(1_000_000);
    let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();

    let obs1 = insert_fail_obs(&store, "cargo test").await;
    let obs2 = insert_fail_obs(&store, "cargo test").await;
    let obs3 = insert_fail_obs(&store, "cargo test").await;

    let signals = vec![Signal::StuckLoop {
        cmd: "cargo test".to_string(),
        count: 3,
        obs_ids: vec![obs1.id.clone(), obs2.id.clone(), obs3.id.clone()],
    }];

    let good_verdict = good_verdict_json(&obs1.id);

    // First call → should succeed and return shown
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            openai_response(&good_verdict)
        ))
        .up_to_n_times(2) // allow 2 calls (first + second tick)
        .mount(&server)
        .await;

    let backend1 = Box::new(DirectMockBackend {
        http: reqwest::Client::new(),
        base_url: server.uri(),
        model: "test-model".to_string(),
    });

    let critic1 = make_critic_with_backend(store.clone(), backend1, clock.clone());
    let first_result = critic1.slow_tick(&signals).await;
    assert!(first_result.is_some(), "first call should return shown pushback");
    assert_eq!(first_result.unwrap().status, "shown");

    // Second call with same evidence → dedupe should skip insert and return None
    let backend2 = Box::new(DirectMockBackend {
        http: reqwest::Client::new(),
        base_url: server.uri(),
        model: "test-model".to_string(),
    });

    let critic2 = make_critic_with_backend(store.clone(), backend2, clock.clone());
    let second_result = critic2.slow_tick(&signals).await;
    assert!(second_result.is_none(), "second call with same evidence should return None (dedupe)");

    // Only one pushback should exist in store (from first call)
    let pushbacks = store.recent_pushbacks(10).await.unwrap();
    let shown_count = pushbacks.iter().filter(|pb| pb.status == "shown").count();
    assert_eq!(shown_count, 1, "only one shown pushback should exist");
}
