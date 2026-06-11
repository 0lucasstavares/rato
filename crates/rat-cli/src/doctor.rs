use std::path::Path;

/// Prints [ok]/[warn]/[fail] lines. Always exits 0 — doctor reports, it does not gate.
pub async fn doctor(socket: &Path) -> anyhow::Result<()> {
    match crate::client::Client::connect(socket).await {
        Ok(mut c) => match c.status().await {
            Ok(s) => println!(
                "[ok]   daemon: ratd {} at {} ({} events)",
                s.version,
                socket.display(),
                s.event_count
            ),
            Err(e) => println!("[fail] daemon: connected but status failed: {e}"),
        },
        Err(_) => {
            println!("[warn] daemon: not reachable at {} (is ratd running?)", socket.display())
        }
    }

    let db = rat_core::paths::db_path();
    if db.exists() {
        println!("[ok]   db: {}", db.display());
    } else {
        println!("[warn] db: {} missing (created on first daemon start)", db.display());
    }

    let unit = crate::install::config_home().join("systemd/user/ratd.service");
    if unit.exists() {
        println!("[ok]   systemd: {}", unit.display());
    } else {
        println!("[warn] systemd: unit not installed (run `rat install`)");
    }

    if let Ok(mut c) = crate::client::Client::connect(socket).await {
        if let Ok(m) = c
            .call(rat_proto::methods::MODE_GET, serde_json::Value::Null)
            .await
            .and_then(|v| Ok(serde_json::from_value::<rat_proto::ModeState>(v)?))
        {
            match m.idle_ms {
                Some(idle) => println!("[ok]   mode: {} (idle {}s)", m.mode, idle / 1000),
                None => println!("[warn] mode: {} (no idle probe; using activity fallback)", m.mode),
            }
        }
    }

    for (bin, arg) in [("git", "--version"), ("tmux", "-V")] {
        match std::process::Command::new(bin).arg(arg).output() {
            Ok(o) if o.status.success() => {
                let first = String::from_utf8_lossy(&o.stdout);
                println!("[ok]   {}: {}", bin, first.lines().next().unwrap_or("").trim());
            }
            _ => println!("[warn] {bin}: not found (needed from M1/M4 onward)"),
        }
    }
    Ok(())
}
