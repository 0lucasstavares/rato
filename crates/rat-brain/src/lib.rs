pub mod backend;
pub mod error;
pub mod keys;
pub mod detect;
pub mod governor;
pub mod critic;

mod openai;
mod anthropic;
mod compat;

pub use backend::{
    BackendConfig, ChatBackend, ChatMessage, ChatRequest, ChatResponse, Provider, Role, Route,
    make_backend,
};
pub use error::LlmError;
