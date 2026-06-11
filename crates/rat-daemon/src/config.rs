use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub llm: LlmConfig,
    pub critic: CriticConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub provider: String,
    pub critic_model: Option<String>,
    pub cheap_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CriticConfig {
    pub enabled: bool,
    pub fast_tick_s: u64,
    pub slow_tick_s: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self { provider: "openai".to_string(), critic_model: None, cheap_model: None }
    }
}

impl Default for CriticConfig {
    fn default() -> Self {
        Self { enabled: true, fast_tick_s: 30, slow_tick_s: 300 }
    }
}

impl Config {
    /// Returns the default config path: $XDG_CONFIG_HOME/rato/config.toml
    /// or ~/.config/rato/config.toml if XDG_CONFIG_HOME is not set.
    pub fn default_path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("rato").join("config.toml")
    }

    /// Load config from the given path. If the file does not exist, writes the
    /// defaults and returns them. Ignores parse errors and returns defaults.
    pub fn load_or_init(path: &std::path::Path) -> Config {
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(contents) => {
                    match toml::from_str::<Config>(&contents) {
                        Ok(cfg) => return cfg,
                        Err(e) => {
                            tracing::warn!("config parse error (using defaults): {e}");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("config read error (using defaults): {e}");
                }
            }
        } else {
            // Write defaults
            let defaults = Config::default();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(contents) = toml::to_string(&defaults) {
                let _ = std::fs::write(path, contents);
            }
        }
        Config::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_default_write_and_parse() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        // File doesn't exist — load_or_init creates it with defaults
        let config = Config::load_or_init(&path);
        assert_eq!(config.llm.provider, "openai");
        assert!(config.critic.enabled);
        assert_eq!(config.critic.fast_tick_s, 30);
        assert_eq!(config.critic.slow_tick_s, 300);
        // File now exists
        assert!(path.exists());
        // Load again — same values
        let config2 = Config::load_or_init(&path);
        assert_eq!(config2.llm.provider, "openai");
        assert_eq!(config2.critic.fast_tick_s, 30);
    }

    #[test]
    fn config_custom_values() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[llm]
provider = "anthropic"

[critic]
enabled = false
fast_tick_s = 60
slow_tick_s = 600
"#,
        )
        .unwrap();
        let config = Config::load_or_init(&path);
        assert_eq!(config.llm.provider, "anthropic");
        assert!(!config.critic.enabled);
        assert_eq!(config.critic.fast_tick_s, 60);
        assert_eq!(config.critic.slow_tick_s, 600);
    }
}
