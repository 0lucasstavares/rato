use crate::error::LlmError;
use crate::openai::OpenAiResponsesBackend;
use crate::anthropic::AnthropicBackend;
use crate::compat::OpenRouterBackend;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Provider {
    OpenAi,
    Anthropic,
    OpenRouter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Critic,
    Cheap,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub json_schema: serde_json::Value,
    pub schema_name: String,
    pub route: Route,
    pub purpose: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub json: serde_json::Value,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub model: String,
}

#[async_trait::async_trait]
pub trait ChatBackend: Send + Sync {
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError>;
    fn provider(&self) -> Provider;
    fn model_for(&self, route: Route) -> &str;
}

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub provider: Provider,
    pub base_url: Option<String>,
    pub critic_model: Option<String>,
    pub cheap_model: Option<String>,
}

pub fn make_backend(cfg: &BackendConfig, key: String) -> Box<dyn ChatBackend> {
    match cfg.provider {
        Provider::OpenAi => Box::new(OpenAiResponsesBackend::new(cfg, key)),
        Provider::Anthropic => Box::new(AnthropicBackend::new(cfg, key)),
        Provider::OpenRouter => Box::new(OpenRouterBackend::new(cfg, key)),
    }
}
