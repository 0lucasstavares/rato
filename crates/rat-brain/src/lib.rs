pub mod backend;
pub mod critic;
pub mod detect;
pub mod error;
pub mod governor;
pub mod keys;

mod anthropic;
mod compat;
mod openai;

pub use backend::{
    make_backend, BackendConfig, ChatBackend, ChatMessage, ChatRequest, ChatResponse, Provider,
    Role, Route,
};
pub use error::LlmError;
