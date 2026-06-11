use serde_json::{json, Value};
use crate::backend::{BackendConfig, ChatBackend, ChatRequest, ChatResponse, Provider, Role, Route};
use crate::error::LlmError;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_CRITIC_MODEL: &str = "claude-opus-4-8";
const DEFAULT_CHEAP_MODEL: &str = "claude-haiku-4-5";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicBackend {
    base_url: String,
    key: String,
    critic_model: String,
    cheap_model: String,
    http: reqwest::Client,
}

impl AnthropicBackend {
    pub fn new(cfg: &BackendConfig, key: String) -> Self {
        Self {
            base_url: cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            key,
            critic_model: cfg.critic_model.clone().unwrap_or_else(|| DEFAULT_CRITIC_MODEL.to_string()),
            cheap_model: cfg.cheap_model.clone().unwrap_or_else(|| DEFAULT_CHEAP_MODEL.to_string()),
            http: reqwest::Client::new(),
        }
    }

    fn build_body(&self, req: &ChatRequest, model: &str) -> Value {
        // Anthropic messages: system is top-level, messages excludes system role
        let messages: Vec<Value> = req.messages.iter().filter_map(|m| {
            let role = match m.role {
                Role::System => return None, // system goes at top level
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            Some(json!({ "role": role, "content": m.content }))
        }).collect();

        json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": messages,
            "output_config": {
                "format": {
                    "type": "json_schema",
                    "schema": req.json_schema
                }
            }
        })
    }

    async fn do_request(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let url = format!("{}/v1/messages", self.base_url);
        let resp = self.http
            .post(&url)
            .header("x-api-key", &self.key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await?;
        Ok(resp)
    }
}

#[async_trait::async_trait]
impl ChatBackend for AnthropicBackend {
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let model = self.model_for(req.route).to_string();
        let body = self.build_body(req, &model);

        let resp = self.do_request(&body).await?;
        let status = resp.status();

        if status.as_u16() == 429 || status.is_server_error() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let resp2 = self.do_request(&body).await?;
            let status2 = resp2.status();
            if !status2.is_success() {
                let snippet = resp2.text().await.unwrap_or_default();
                let snippet: String = snippet.chars().take(200).collect();
                return Err(LlmError::Http(status2.as_u16(), snippet));
            }
            return parse_anthropic_response(resp2, &model).await;
        }

        if !status.is_success() {
            let snippet = resp.text().await.unwrap_or_default();
            let snippet: String = snippet.chars().take(200).collect();
            return Err(LlmError::Http(status.as_u16(), snippet));
        }

        parse_anthropic_response(resp, &model).await
    }

    fn provider(&self) -> Provider {
        Provider::Anthropic
    }

    fn model_for(&self, route: Route) -> &str {
        match route {
            Route::Critic => &self.critic_model,
            Route::Cheap => &self.cheap_model,
        }
    }
}

async fn parse_anthropic_response(resp: reqwest::Response, model: &str) -> Result<ChatResponse, LlmError> {
    let val: Value = resp.json().await?;

    // Check for refusal
    if val["stop_reason"].as_str() == Some("refusal") {
        return Err(LlmError::Refused);
    }

    // text from content[0].text
    let text = val["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["text"].as_str())
        .ok_or_else(|| LlmError::Http(0, "no content[0].text in Anthropic response".to_string()))?;

    let json: Value = serde_json::from_str(text)?;
    let tokens_in = val["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
    let tokens_out = val["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

    Ok(ChatResponse {
        json,
        tokens_in,
        tokens_out,
        model: model.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendConfig, ChatMessage, ChatRequest, ChatBackend, Provider, Route, Role};
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path, header, body_json};
    use serde_json::json;

    fn make_backend(server: &MockServer) -> AnthropicBackend {
        let cfg = BackendConfig {
            provider: Provider::Anthropic,
            base_url: Some(server.uri()),
            critic_model: None,
            cheap_model: None,
        };
        AnthropicBackend::new(&cfg, "test-key".to_string())
    }

    fn make_request() -> ChatRequest {
        ChatRequest {
            system: "You are a helpful assistant.".to_string(),
            messages: vec![
                ChatMessage { role: Role::User, content: "Hello".to_string() },
            ],
            json_schema: json!({ "type": "object", "properties": { "result": { "type": "string" } }, "required": ["result"], "additionalProperties": false }),
            schema_name: "test_schema".to_string(),
            route: Route::Critic,
            purpose: "test".to_string(),
            max_tokens: 256,
        }
    }

    fn realistic_anthropic_response() -> Value {
        json!({
            "id": "msg_abc123",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-8",
            "stop_reason": "end_turn",
            "content": [
                {
                    "type": "text",
                    "text": "{\"result\": \"hello world\"}"
                }
            ],
            "usage": {
                "input_tokens": 55,
                "output_tokens": 12
            }
        })
    }

    #[tokio::test]
    async fn happy_path_full_request_shape() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        let expected_body = json!({
            "model": "claude-opus-4-8",
            "max_tokens": 256,
            "system": "You are a helpful assistant.",
            "messages": [{ "role": "user", "content": "Hello" }],
            "output_config": {
                "format": {
                    "type": "json_schema",
                    "schema": { "type": "object", "properties": { "result": { "type": "string" } }, "required": ["result"], "additionalProperties": false }
                }
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(body_json(&expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(realistic_anthropic_response()))
            .mount(&server)
            .await;

        let resp = backend.complete(&req).await.unwrap();
        assert_eq!(resp.json["result"], "hello world");
        assert_eq!(resp.tokens_in, 55);
        assert_eq!(resp.tokens_out, 12);
        assert_eq!(resp.model, "claude-opus-4-8");
    }

    #[tokio::test]
    async fn retry_on_429_then_200() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(realistic_anthropic_response()))
            .mount(&server)
            .await;

        let resp = backend.complete(&req).await.unwrap();
        assert_eq!(resp.json["result"], "hello world");
    }

    #[tokio::test]
    async fn stop_reason_refusal_returns_refused() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        let refusal_resp = json!({
            "id": "msg_refusal",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-8",
            "stop_reason": "refusal",
            "content": [],
            "usage": { "input_tokens": 10, "output_tokens": 0 }
        });

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(refusal_resp))
            .mount(&server)
            .await;

        let err = backend.complete(&req).await.unwrap_err();
        assert!(matches!(err, LlmError::Refused));
    }

    #[tokio::test]
    async fn malformed_json_returns_bad_json() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        let bad_resp = json!({
            "id": "msg_abc",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-8",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "not valid json {" }],
            "usage": { "input_tokens": 10, "output_tokens": 3 }
        });

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(bad_resp))
            .mount(&server)
            .await;

        let err = backend.complete(&req).await.unwrap_err();
        assert!(matches!(err, LlmError::BadJson(_)));
    }

    #[tokio::test]
    async fn model_routing_defaults() {
        let cfg = BackendConfig {
            provider: Provider::Anthropic,
            base_url: None,
            critic_model: None,
            cheap_model: None,
        };
        let backend = AnthropicBackend::new(&cfg, "key".to_string());
        assert_eq!(backend.model_for(Route::Critic), "claude-opus-4-8");
        assert_eq!(backend.model_for(Route::Cheap), "claude-haiku-4-5");
    }

    #[tokio::test]
    async fn model_routing_override() {
        let cfg = BackendConfig {
            provider: Provider::Anthropic,
            base_url: None,
            critic_model: Some("claude-custom-critic".to_string()),
            cheap_model: Some("claude-custom-cheap".to_string()),
        };
        let backend = AnthropicBackend::new(&cfg, "key".to_string());
        assert_eq!(backend.model_for(Route::Critic), "claude-custom-critic");
        assert_eq!(backend.model_for(Route::Cheap), "claude-custom-cheap");
    }
}
