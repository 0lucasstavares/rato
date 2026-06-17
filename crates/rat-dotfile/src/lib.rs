//! Deterministic DotfileEditor core for M7.
//!
//! This crate is the write chokepoint logic without daemon/RPC wiring: validate
//! bytes first, reject missing MCP commands, apply by same-directory temp file
//! plus rename, and support byte-exact revert.

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    Json,
    Jsonc,
    Toml,
    Yaml,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotfileSnapshot {
    pub path: PathBuf,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub diff: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DotfileError {
    #[error("invalid utf-8")]
    InvalidUtf8,
    #[error("invalid json: {0}")]
    InvalidJson(String),
    #[error("invalid toml: {0}")]
    InvalidToml(String),
    #[error("invalid yaml: {0}")]
    InvalidYaml(String),
    #[error("unsupported config kind for validation")]
    UnsupportedKind,
    #[error("mcp command not found: {0}")]
    MissingMcpCommand(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub trait CommandResolver {
    fn command_exists(&self, command: &str) -> bool;
}

pub struct PathCommandResolver;

impl CommandResolver for PathCommandResolver {
    fn command_exists(&self, command: &str) -> bool {
        if command.contains('/') {
            return Path::new(command).is_file();
        }
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&path).any(|dir| dir.join(command).is_file())
    }
}

pub fn validate(
    kind: ConfigKind,
    bytes: &[u8],
    commands: &dyn CommandResolver,
) -> Result<(), DotfileError> {
    match kind {
        ConfigKind::Json => {
            let value = parse_json(bytes)?;
            validate_mcp_commands(&value, commands)
        }
        ConfigKind::Jsonc => {
            let text = std::str::from_utf8(bytes).map_err(|_| DotfileError::InvalidUtf8)?;
            let stripped = strip_jsonc_comments(text);
            let value = serde_json::from_str::<Value>(&stripped)
                .map_err(|e| DotfileError::InvalidJson(e.to_string()))?;
            validate_mcp_commands(&value, commands)
        }
        ConfigKind::Toml => {
            let text = std::str::from_utf8(bytes).map_err(|_| DotfileError::InvalidUtf8)?;
            toml::from_str::<toml::Value>(text)
                .map_err(|e| DotfileError::InvalidToml(e.to_string()))?;
            Ok(())
        }
        ConfigKind::Yaml => {
            serde_yaml::from_slice::<serde_yaml::Value>(bytes)
                .map_err(|e| DotfileError::InvalidYaml(e.to_string()))?;
            Ok(())
        }
        ConfigKind::Text => {
            std::str::from_utf8(bytes).map_err(|_| DotfileError::InvalidUtf8)?;
            Ok(())
        }
    }
}

pub fn apply_atomic(
    path: &Path,
    kind: ConfigKind,
    after: &[u8],
    commands: &dyn CommandResolver,
) -> Result<DotfileSnapshot, DotfileError> {
    validate(kind, after, commands)?;
    let before = fs::read(path).unwrap_or_default();
    atomic_write(path, after)?;
    Ok(DotfileSnapshot {
        path: path.to_path_buf(),
        diff: simple_diff(&before, after),
        before,
        after: after.to_vec(),
    })
}

pub fn revert(snapshot: &DotfileSnapshot) -> Result<DotfileSnapshot, DotfileError> {
    let current = fs::read(&snapshot.path).unwrap_or_default();
    atomic_write(&snapshot.path, &snapshot.before)?;
    Ok(DotfileSnapshot {
        path: snapshot.path.clone(),
        diff: simple_diff(&current, &snapshot.before),
        before: current,
        after: snapshot.before.clone(),
    })
}

fn parse_json(bytes: &[u8]) -> Result<Value, DotfileError> {
    serde_json::from_slice(bytes).map_err(|e| DotfileError::InvalidJson(e.to_string()))
}

fn validate_mcp_commands(
    value: &Value,
    commands: &dyn CommandResolver,
) -> Result<(), DotfileError> {
    walk_json(value, &mut |key, value| {
        if key == "command" {
            if let Some(command) = value.as_str() {
                if !commands.command_exists(command) {
                    return Err(DotfileError::MissingMcpCommand(command.to_string()));
                }
            }
        }
        Ok(())
    })
}

fn walk_json(
    value: &Value,
    f: &mut dyn FnMut(&str, &Value) -> Result<(), DotfileError>,
) -> Result<(), DotfileError> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                f(key, child)?;
                walk_json(child, f)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                walk_json(child, f)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DotfileError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dotfile");
    let tmp_path = parent.join(format!(".{file_name}.rato-tmp-{}", std::process::id()));
    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn strip_jsonc_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            for next in chars.by_ref() {
                if prev == '*' && next == '/' {
                    break;
                }
                prev = next;
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn simple_diff(before: &[u8], after: &[u8]) -> String {
    if before == after {
        return String::new();
    }
    let before = String::from_utf8_lossy(before);
    let after = String::from_utf8_lossy(after);
    format!(
        "--- before\n+++ after\n-{}\n+{}\n",
        before.trim_end(),
        after.trim_end()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct FakeCommands {
        present: HashSet<String>,
    }

    impl FakeCommands {
        fn new(commands: &[&str]) -> Self {
            Self {
                present: commands.iter().map(|command| command.to_string()).collect(),
            }
        }
    }

    impl CommandResolver for FakeCommands {
        fn command_exists(&self, command: &str) -> bool {
            self.present.contains(command)
        }
    }

    #[test]
    fn invalid_json_rejects_before_write() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(&path, b"{\"old\":true}\n").unwrap();

        let err = apply_atomic(
            &path,
            ConfigKind::Json,
            b"{not-json",
            &FakeCommands::new(&[]),
        )
        .unwrap_err();

        assert!(matches!(err, DotfileError::InvalidJson(_)));
        assert_eq!(fs::read(&path).unwrap(), b"{\"old\":true}\n");
    }

    #[test]
    fn missing_mcp_command_rejects_before_write() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let bytes = br#"{"mcpServers":{"tool":{"command":"missing-tool"}}}"#;

        let err =
            apply_atomic(&path, ConfigKind::Json, bytes, &FakeCommands::new(&[])).unwrap_err();

        assert!(
            matches!(err, DotfileError::MissingMcpCommand(command) if command == "missing-tool")
        );
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
    }

    #[test]
    fn jsonc_comments_are_accepted_and_commands_checked() {
        let bytes = br#"{
          // local server
          "mcpServers": {"tool": {"command": "known-tool"}}
        }"#;

        validate(
            ConfigKind::Jsonc,
            bytes,
            &FakeCommands::new(&["known-tool"]),
        )
        .unwrap();
    }

    #[test]
    fn malformed_toml_rejects_before_write() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        fs::write(&path, b"[old]\nvalue = true\n").unwrap();

        let err = apply_atomic(
            &path,
            ConfigKind::Toml,
            b"[new\nvalue = false\n",
            &FakeCommands::new(&[]),
        )
        .unwrap_err();

        assert!(matches!(err, DotfileError::InvalidToml(_)));
        assert_eq!(fs::read(&path).unwrap(), b"[old]\nvalue = true\n");
    }

    #[test]
    fn malformed_yaml_rejects_before_write() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.yaml");
        fs::write(&path, b"old: true\n").unwrap();

        let err = apply_atomic(
            &path,
            ConfigKind::Yaml,
            b"new: [unterminated\n",
            &FakeCommands::new(&[]),
        )
        .unwrap_err();

        assert!(matches!(err, DotfileError::InvalidYaml(_)));
        assert_eq!(fs::read(&path).unwrap(), b"old: true\n");
    }

    #[test]
    fn apply_and_revert_restore_exact_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        let before = b"{\"old\":true}\n";
        let after = b"{\"old\":false,\"command\":\"known-tool\"}\n";
        fs::write(&path, before).unwrap();

        let snapshot = apply_atomic(
            &path,
            ConfigKind::Json,
            after,
            &FakeCommands::new(&["known-tool"]),
        )
        .unwrap();
        assert_eq!(snapshot.before, before);
        assert_eq!(snapshot.after, after);
        assert_eq!(fs::read(&path).unwrap(), after);

        let reverted = revert(&snapshot).unwrap();
        assert_eq!(reverted.after, before);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
