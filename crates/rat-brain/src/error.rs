use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("HTTP {0}: {1}")]
    Http(u16, String),

    #[error("model refused to answer")]
    Refused,

    #[error("bad JSON from model: {0}")]
    BadJson(#[from] serde_json::Error),

    #[error("no key configured for provider {0}")]
    MissingKey(String),

    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
}
