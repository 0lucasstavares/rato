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

/// Run a keyring operation on a fresh OS thread.
///
/// keyring's async-secret-service backend drives zbus with an internal
/// `block_on`, which panics if called from inside a tokio runtime ("cannot
/// start a runtime from within a runtime"). Every caller of this module is
/// inside #[tokio::main], so each keyring op hops to a plain thread where
/// zbus can spin its own executor. Key ops are rare (startup + `rat setup`),
/// so the thread spawn cost is irrelevant.
fn off_runtime<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match std::thread::spawn(f).join() {
        Ok(v) => v,
        Err(_) => panic!("keyring thread panicked"),
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
    let name = provider_name(&p).to_string();
    off_runtime(move || match keyring::Entry::new(keyring_service(), keyring_account(&p)) {
        Ok(entry) => entry.get_password().map_err(|_| LlmError::MissingKey(name)),
        Err(_) => Err(LlmError::MissingKey(name)),
    })
}

/// Store the API key for the given provider in the system keyring.
pub fn set_key(p: Provider, value: &str) -> Result<(), LlmError> {
    let name = provider_name(&p).to_string();
    let value = value.to_string();
    off_runtime(move || {
        let entry = keyring::Entry::new(keyring_service(), keyring_account(&p))
            .map_err(|_| LlmError::MissingKey(name.clone()))?;
        entry.set_password(&value).map_err(|_| LlmError::MissingKey(name))
    })
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
    off_runtime(move || match keyring::Entry::new(keyring_service(), keyring_account(&p)) {
        Ok(entry) => entry.get_password().is_ok(),
        Err(_) => false,
    })
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
        let key = env_var_name(&Provider::OpenRouter);
        env::remove_var(key);
        // The machine keyring may legitimately hold rato/openrouter (it does on
        // the operator's machine after `rat setup`), so only assert the error
        // shape when the keyring genuinely lacks the entry.
        let result = get_key(Provider::OpenRouter);
        if key_present(Provider::OpenRouter) {
            assert!(result.is_ok(), "key present in keyring → get_key must return it");
        } else {
            assert!(matches!(result, Err(LlmError::MissingKey(_))));
        }
    }
}
