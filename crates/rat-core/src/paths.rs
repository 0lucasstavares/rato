use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// $XDG_RUNTIME_DIR/rato, falling back to /tmp/rato-<uid>.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => PathBuf::from(d).join("rato"),
        None => PathBuf::from(format!("/tmp/rato-{}", unsafe { libc::getuid() })),
    }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("ratd.sock")
}

/// $XDG_DATA_HOME/rato, falling back to ~/.local/share/rato.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) => PathBuf::from(d).join("rato"),
        None => {
            PathBuf::from(std::env::var_os("HOME").expect("HOME not set")).join(".local/share/rato")
        }
    }
}

/// $XDG_STATE_HOME/rato, falling back to ~/.local/state/rato.
pub fn state_dir() -> PathBuf {
    match std::env::var_os("XDG_STATE_HOME") {
        Some(d) => PathBuf::from(d).join("rato"),
        None => {
            PathBuf::from(std::env::var_os("HOME").expect("HOME not set")).join(".local/state/rato")
        }
    }
}

pub fn db_path() -> PathBuf {
    data_dir().join("rato.db")
}

/// Create a directory (and parents) and clamp it to 0700.
pub fn ensure_private_dir(p: &Path) -> io::Result<()> {
    std::fs::create_dir_all(p)?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_lives_under_runtime_dir() {
        let s = socket_path();
        assert!(
            s.ends_with("rato/ratd.sock") || s.to_string_lossy().contains("/tmp/rato-"),
            "unexpected socket path: {}",
            s.display()
        );
    }

    #[test]
    fn db_lives_under_data_dir() {
        assert!(db_path().ends_with("rato/rato.db"));
    }

    #[test]
    fn state_lives_under_state_dir() {
        assert!(state_dir().ends_with("rato"));
    }
}
