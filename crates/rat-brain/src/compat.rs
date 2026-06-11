use serde_json::{json, Value};
use crate::backend::{BackendConfig, ChatBackend, ChatRequest, ChatResponse, Provider, Role, Route};
use crate::error::LlmError;

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_CRITIC_MODEL: &str = "openai/gpt-5.1";
const DEFAULT_CHEAP_MODEL: &str = "openai/gpt-5-mini";

pub struct OpenRouterBackend {
    base_url: String,
    key: String,
    critic_model: String,
    cheap_model: String,
    http: reqwest::Client,
}

impl OpenRouterBackend {
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
        // OpenRouter / OpenAI-compat: system message first, then the rest
        let mut messages: Vec<Value> = vec![
            json!({ "role": "system", "content": req.system })
        ];
        for m in &req.messages {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            messages.push(json!({ "role": role, "content": m.content }));
        }

        json!({
            "model": model,
            "messages": messages,
            "max_tokens": req.max_tokens,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": req.schema_name,
                    "schema": req.json_schema,
                    "strict": true
                }
            }
        })
    }

    async fn do_request(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.http
            .post(&url)
            .bearer_auth(&self.key)
            .json(body)
            .send()
            .await?;
        Ok(resp)
    }
}

#[async_trait::async_trait]
impl ChatBackend for OpenRouterBackend {
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
            return parse_compat_response(resp2, &model).await;
        }

        if !status.is_success() {
            let snippet = resp.text().await.unwrap_or_default();
            let snippet: String = snippet.chars().take(200).collect();
            return Err(LlmError::Http(status.as_u16(), snippet));
        }

        parse_compat_response(resp, &model).await
    }

    fn provider(&self) -> Provider {
        Provider::OpenRouter
    }

    fn model_for(&self, route: Route) -> &str {
        match route {
            Route::Critic => &self.critic_model,
            Route::Cheap => &self.cheap_model,
        }
    }
}

async fn parse_compat_response(resp: reqwest::Response, model: &str) -> Result<ChatResponse, LlmError> {
    let val: Value = resp.json().await?;

    // text from choices[0].message.content
    let text = val["choices"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["message"]["content"].as_str())
        .ok_or_else(|| LlmError::Http(0, "no choices[0].message.content in response".to_string()))?;

    let json: Value = serde_json::from_str(text)?;
    let tokens_in = val["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
    let tokens_out = val["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;

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
    use wiremock::matchers::{method, path, body_json};
    use serde_json::json;

    fn make_backend(server: &MockServer) -> OpenRouterBackend {
        let cfg = BackendConfig {
            provider: Provider::OpenRouter,
            base_url: Some(server.uri()),
            critic_model: None,
            cheap_model: None,
        };
        OpenRouterBackend::new(&cfg, "test-key".to_string())
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

    fn realistic_compat_response(model: &str) -> Value {
        json!({
            "id": "chatcmpl-abc123",
            "object": "chat.completion",
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "{\"result\": \"hello world\"}"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 30,
                "completion_tokens": 8,
                "total_tokens": 38
            }
        })
    }

    #[tokio::test]
    async fn happy_path_full_request_shape() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        let expected_body = json!({
            "model": "openai/gpt-5.1",
            "messages": [
                { "role": "system", "content": "You are a helpful assistant." },
                { "role": "user", "content": "Hello" }
            ],
            "max_tokens": 256,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "test_schema",
                    "schema": { "type": "object", "properties": { "result": { "type": "string" } }, "required": ["result"], "additionalProperties": false },
                    "strict": true
                }
            }
        });

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_json(&expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(realistic_compat_response("openai/gpt-5.1")))
            .mount(&server)
            .await;

        let resp = backend.complete(&req).await.unwrap();
        assert_eq!(resp.json["result"], "hello world");
        assert_eq!(resp.tokens_in, 30);
        assert_eq!(resp.tokens_out, 8);
        assert_eq!(resp.model, "openai/gpt-5.1");
    }

    #[tokio::test]
    async fn retry_on_429_then_200() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(realistic_compat_response("openai/gpt-5.1")))
            .mount(&server)
            .await;

        let resp = backend.complete(&req).await.unwrap();
        assert_eq!(resp.json["result"], "hello world");
    }

    #[tokio::test]
    async fn malformed_json_returns_bad_json() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        let bad_resp = json!({
            "id": "chatcmpl-abc",
            "object": "chat.completion",
            "model": "openai/gpt-5.1",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "not valid json {"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": { "prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13 }
        });

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(bad_resp))
            .mount(&server)
            .await;

        let err = backend.complete(&req).await.unwrap_err();
        assert!(matches!(err, LlmError::BadJson(_)));
    }

    #[tokio::test]
    async fn model_routing_defaults() {
        let cfg = BackendConfig {
            provider: Provider::OpenRouter,
            base_url: None,
            critic_model: None,
            cheap_model: None,
        };
        let backend = OpenRouterBackend::new(&cfg, "key".to_string());
        assert_eq!(backend.model_for(Route::Critic), "openai/gpt-5.1");
        assert_eq!(backend.model_for(Route::Cheap), "openai/gpt-5-mini");
    }

    #[tokio::test]
    async fn model_routing_override() {
        let cfg = BackendConfig {
            provider: Provider::OpenRouter,
            base_url: None,
            critic_model: Some("custom/critic".to_string()),
            cheap_model: Some("custom/cheap".to_string()),
        };
        let backend = OpenRouterBackend::new(&cfg, "key".to_string());
        assert_eq!(backend.model_for(Route::Critic), "custom/critic");
        assert_eq!(backend.model_for(Route::Cheap), "custom/cheap");
    }
}
