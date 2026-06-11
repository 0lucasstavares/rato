use crate::backend::Provider;
use crate::error::LlmError;

fn env_var_name(p: &Provider) -> &'static str {
    match p {
        Provider::OpenAi => "RATO_OPENAI_KEY",
        Provider::Anthropic => "RATO_ANTHROPIC_KEY",
        Provider::OpenRouter => "RATO_OPENROUTER_KEY",
    }
}

fn keyring_service() -> &'static str {
    "rato"
}

fn keyring_account(p: &Provider) -> &'static str {
    match p {
        Provider::OpenAi => "openai",
        Provider::Anthropic => "anthropic",
        Provider::OpenRouter => "openrouter",
    }
}

fn provider_name(p: &Provider) -> &'static str {
    match p {
        Provider::OpenAi => "openai",
        Provider::Anthropic => "anthropic",
        Provider::OpenRouter => "openrouter",
    }
}

/// Retrieve the API key for a provider from the environment variable.
///
/// Returns None if the variable is not set or is empty.
fn key_from_env(p: &Provider) -> Option<String> {
    match std::env::var(env_var_name(p)) {
        Ok(val) if !val.is_empty() => Some(val),
        _ => None,
    }
}

/// Get the API key for the given provider.
///
/// Checks env override first (useful for tests/CI), then falls back to the
/// system keyring (Secret Service on Linux).
pub fn get_key(p: Provider) -> Result<String, LlmError> {
    // 1. env override
    if let Some(val) = key_from_env(&p) {
        return Ok(val);
    }

    // 2. keyring
    match keyring::Entry::new(keyring_service(), keyring_account(&p)) {
        Ok(entry) => {
            entry.get_password().map_err(|_| LlmError::MissingKey(provider_name(&p).to_string()))
        }
        Err(_) => Err(LlmError::MissingKey(provider_name(&p).to_string())),
    }
}

/// Store the API key for the given provider in the system keyring.
pub fn set_key(p: Provider, value: &str) -> Result<(), LlmError> {
    let entry = keyring::Entry::new(keyring_service(), keyring_account(&p))
        .map_err(|_| LlmError::MissingKey(provider_name(&p).to_string()))?;
    entry.set_password(value)
        .map_err(|_| LlmError::MissingKey(provider_name(&p).to_string()))
}

/// Check whether a key is available (env or keyring), without erroring.
pub fn key_present(p: Provider) -> bool {
    // env override
    if let Ok(val) = std::env::var(env_var_name(&p)) {
        if !val.is_empty() {
            return true;
        }
    }

    // keyring
    match keyring::Entry::new(keyring_service(), keyring_account(&p)) {
        Ok(entry) => entry.get_password().is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn env_override_openai() {
        let key = env_var_name(&Provider::OpenAi);
        env::set_var(key, "sk-test-openai");
        let result = get_key(Provider::OpenAi).unwrap();
        assert_eq!(result, "sk-test-openai");
        env::remove_var(key);
    }

    #[test]
    fn env_override_anthropic() {
        let key = env_var_name(&Provider::Anthropic);
        env::set_var(key, "sk-ant-test");
        let result = get_key(Provider::Anthropic).unwrap();
        assert_eq!(result, "sk-ant-test");
        env::remove_var(key);
    }

    #[test]
    fn env_override_openrouter() {
        let key = env_var_name(&Provider::OpenRouter);
        env::set_var(key, "sk-or-test");
        let result = get_key(Provider::OpenRouter).unwrap();
        assert_eq!(result, "sk-or-test");
        env::remove_var(key);
    }

    #[test]
    fn key_present_via_env() {
        let key = env_var_name(&Provider::Anthropic);
        env::remove_var(key);
        env::set_var(key, "test-ant-key");
        assert!(key_present(Provider::Anthropic));
        env::remove_var(key);
    }

    #[test]
    fn key_from_env_helper() {
        let key = env_var_name(&Provider::OpenAi);
        env::remove_var(key);
        assert_eq!(key_from_env(&Provider::OpenAi), None);
        env::set_var(key, "sk-test");
        assert_eq!(key_from_env(&Provider::OpenAi), Some("sk-test".to_string()));
        env::remove_var(key);
    }

    #[test]
    fn missing_key_returns_error() {
        // Use a distinct provider to avoid keyring conflicts
        let key = env_var_name(&Provider::OpenRouter);
        env::remove_var(key);
        let result = get_key(Provider::OpenRouter);
        // Assert the error is specifically MissingKey, not just any error
        assert!(matches!(result, Err(LlmError::MissingKey(_))));
    }
}
