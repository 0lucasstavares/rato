use crate::backend::{
    BackendConfig, ChatBackend, ChatRequest, ChatResponse, Provider, Role, Route,
};
use crate::error::LlmError;
use serde_json::{json, Value};

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_CRITIC_MODEL: &str = "gpt-5.1";
const DEFAULT_CHEAP_MODEL: &str = "gpt-5-mini";

pub struct OpenAiResponsesBackend {
    base_url: String,
    key: String,
    critic_model: String,
    cheap_model: String,
    http: reqwest::Client,
}

impl OpenAiResponsesBackend {
    pub fn new(cfg: &BackendConfig, key: String) -> Self {
        Self {
            base_url: cfg
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            key,
            critic_model: cfg
                .critic_model
                .clone()
                .unwrap_or_else(|| DEFAULT_CRITIC_MODEL.to_string()),
            cheap_model: cfg
                .cheap_model
                .clone()
                .unwrap_or_else(|| DEFAULT_CHEAP_MODEL.to_string()),
            http: reqwest::Client::new(),
        }
    }

    fn build_body(&self, req: &ChatRequest, model: &str) -> Value {
        let input: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                json!({ "role": role, "content": m.content })
            })
            .collect();

        json!({
            "model": model,
            "instructions": req.system,
            "input": input,
            "max_output_tokens": req.max_tokens,
            "store": false,
            "metadata": { "purpose": req.purpose },
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": req.schema_name,
                    "schema": req.json_schema,
                    "strict": true
                }
            }
        })
    }

    async fn do_request(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let url = format!("{}/v1/responses", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.key)
            .json(body)
            .send()
            .await?;
        Ok(resp)
    }
}

#[async_trait::async_trait]
impl ChatBackend for OpenAiResponsesBackend {
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let model = self.model_for(req.route).to_string();
        let body = self.build_body(req, &model);

        let resp = self.do_request(&body).await?;
        let status = resp.status();

        if status.as_u16() == 429 || status.is_server_error() {
            // one retry after 2s
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let resp2 = self.do_request(&body).await?;
            let status2 = resp2.status();
            if !status2.is_success() {
                let snippet = resp2.text().await.unwrap_or_default();
                let snippet: String = snippet.chars().take(200).collect();
                return Err(LlmError::Http(status2.as_u16(), snippet));
            }
            return parse_openai_response(resp2, &model).await;
        }

        if !status.is_success() {
            let snippet = resp.text().await.unwrap_or_default();
            let snippet: String = snippet.chars().take(200).collect();
            return Err(LlmError::Http(status.as_u16(), snippet));
        }

        parse_openai_response(resp, &model).await
    }

    fn provider(&self) -> Provider {
        Provider::OpenAi
    }

    fn model_for(&self, route: Route) -> &str {
        match route {
            Route::Critic => &self.critic_model,
            Route::Cheap => &self.cheap_model,
        }
    }
}

async fn parse_openai_response(
    resp: reqwest::Response,
    model: &str,
) -> Result<ChatResponse, LlmError> {
    let val: Value = resp.json().await?;

    // Extract text from output[].content[] where type == "output_text"
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
        .ok_or_else(|| LlmError::Http(0, "no output_text found in response".to_string()))?;

    let json: Value = serde_json::from_str(&text)?;
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
    use crate::backend::{
        BackendConfig, ChatBackend, ChatMessage, ChatRequest, Provider, Role, Route,
    };
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_backend(server: &MockServer) -> OpenAiResponsesBackend {
        let cfg = BackendConfig {
            provider: Provider::OpenAi,
            base_url: Some(server.uri()),
            critic_model: None,
            cheap_model: None,
        };
        OpenAiResponsesBackend::new(&cfg, "test-key".to_string())
    }

    fn make_request() -> ChatRequest {
        ChatRequest {
            system: "You are a helpful assistant.".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            json_schema: json!({ "type": "object", "properties": { "result": { "type": "string" } }, "required": ["result"], "additionalProperties": false }),
            schema_name: "test_schema".to_string(),
            route: Route::Critic,
            purpose: "test".to_string(),
            max_tokens: 256,
        }
    }

    fn realistic_openai_response(model: &str) -> Value {
        json!({
            "id": "resp_abc123",
            "object": "response",
            "model": model,
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "{\"result\": \"hello world\"}"
                        }
                    ]
                }
            ],
            "usage": {
                "input_tokens": 42,
                "output_tokens": 10
            }
        })
    }

    #[tokio::test]
    async fn happy_path_full_request_shape() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        let expected_body = json!({
            "model": "gpt-5.1",
            "instructions": "You are a helpful assistant.",
            "input": [{ "role": "user", "content": "Hello" }],
            "max_output_tokens": 256,
            "store": false,
            "metadata": { "purpose": "test" },
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "test_schema",
                    "schema": { "type": "object", "properties": { "result": { "type": "string" } }, "required": ["result"], "additionalProperties": false },
                    "strict": true
                }
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(body_json(&expected_body))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(realistic_openai_response("gpt-5.1")),
            )
            .mount(&server)
            .await;

        let resp = backend.complete(&req).await.unwrap();
        assert_eq!(resp.json["result"], "hello world");
        assert_eq!(resp.tokens_in, 42);
        assert_eq!(resp.tokens_out, 10);
        assert_eq!(resp.model, "gpt-5.1");
    }

    #[tokio::test]
    async fn retry_on_429_then_200() {
        let server = MockServer::start().await;
        let backend = make_backend(&server);
        let req = make_request();

        // First call → 429
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Second call → 200
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(realistic_openai_response("gpt-5.1")),
            )
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
            "id": "resp_abc123",
            "object": "response",
            "model": "gpt-5.1",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "not valid json {"
                        }
                    ]
                }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(bad_resp))
            .mount(&server)
            .await;

        let err = backend.complete(&req).await.unwrap_err();
        assert!(matches!(err, LlmError::BadJson(_)));
    }

    #[tokio::test]
    async fn model_routing_defaults() {
        let cfg = BackendConfig {
            provider: Provider::OpenAi,
            base_url: None,
            critic_model: None,
            cheap_model: None,
        };
        let backend = OpenAiResponsesBackend::new(&cfg, "key".to_string());
        assert_eq!(backend.model_for(Route::Critic), "gpt-5.1");
        assert_eq!(backend.model_for(Route::Cheap), "gpt-5-mini");
    }

    #[tokio::test]
    async fn model_routing_override() {
        let cfg = BackendConfig {
            provider: Provider::OpenAi,
            base_url: None,
            critic_model: Some("gpt-custom-critic".to_string()),
            cheap_model: Some("gpt-custom-cheap".to_string()),
        };
        let backend = OpenAiResponsesBackend::new(&cfg, "key".to_string());
        assert_eq!(backend.model_for(Route::Critic), "gpt-custom-critic");
        assert_eq!(backend.model_for(Route::Cheap), "gpt-custom-cheap");
    }
}
