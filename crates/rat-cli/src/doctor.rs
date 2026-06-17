use std::path::Path;

/// Prints [ok]/[warn]/[fail] lines. Always exits 0 — doctor reports, it does not gate.
pub async fn doctor(socket: &Path) -> anyhow::Result<()> {
    match rat_client::Client::connect(socket).await {
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
            println!(
                "[warn] daemon: not reachable at {} (is ratd running?)",
                socket.display()
            )
        }
    }

    let db = rat_core::paths::db_path();
    if db.exists() {
        println!("[ok]   db: {}", db.display());
    } else {
        println!(
            "[warn] db: {} missing (created on first daemon start)",
            db.display()
        );
    }

    let unit = crate::install::config_home().join("systemd/user/ratd.service");
    if unit.exists() {
        println!("[ok]   systemd: {}", unit.display());
    } else {
        println!("[warn] systemd: unit not installed (run `rat install`)");
    }

    if let Ok(mut c) = rat_client::Client::connect(socket).await {
        if let Ok(m) = c
            .call(rat_proto::methods::MODE_GET, serde_json::Value::Null)
            .await
            .and_then(|v| Ok(serde_json::from_value::<rat_proto::ModeState>(v)?))
        {
            match m.idle_ms {
                Some(idle) => println!("[ok]   mode: {} (idle {}s)", m.mode, idle / 1000),
                None => println!(
                    "[warn] mode: {} (no idle probe; using activity fallback)",
                    m.mode
                ),
            }
        }
    }

    // M5: screen/ocr sensor health, via the `status` RPC's `sensors` field.
    if let Ok(mut c) = rat_client::Client::connect(socket).await {
        match c.status().await {
            Ok(s) if !s.sensors.is_empty() => {
                for sensor in &s.sensors {
                    match sensor.state.as_str() {
                        "ok" => println!("[ok]   sensor {}: ok", sensor.name),
                        _ => println!(
                            "[warn] sensor {}: unavailable ({})",
                            sensor.name,
                            sensor.reason.as_deref().unwrap_or("unknown reason")
                        ),
                    }
                }
            }
            Ok(_) => println!("[warn] sensors: not reported (older daemon?)"),
            Err(e) => println!("[fail] sensors: status query failed: {e}"),
        }
    }

    // M5: ring dir occupancy and pin count.
    if let Ok(mut c) = rat_client::Client::connect(socket).await {
        match c
            .call(rat_proto::methods::RING_STATUS, serde_json::Value::Null)
            .await
            .and_then(|v| Ok(serde_json::from_value::<Vec<rat_proto::RingMediaStatusDto>>(v)?))
        {
            Ok(rows) => {
                let total: u32 = rows.iter().map(|r| r.segment_count).sum();
                let detail = rows
                    .iter()
                    .map(|r| format!("{}={}", r.media, r.segment_count))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("[ok]   ring: {total} segment(s) ({detail})");
            }
            Err(e) => println!("[warn] ring: status query failed ({e})"),
        }
    } else {
        let ring_dir = rat_core::paths::state_dir().join("ring");
        if ring_dir.exists() {
            let segment_count = count_ring_segments(&ring_dir);
            println!(
                "[ok]   ring: {} ({} segment(s))",
                ring_dir.display(),
                segment_count
            );
        } else {
            println!(
                "[warn] ring: {} missing (created on first capture tick)",
                ring_dir.display()
            );
        }
    }

    if let Ok(mut c) = rat_client::Client::connect(socket).await {
        match c
            .call(rat_proto::methods::PINS_LIST, serde_json::Value::Null)
            .await
            .and_then(|v| Ok(serde_json::from_value::<Vec<rat_proto::PinDto>>(v)?))
        {
            Ok(pins) => println!("[ok]   pins: {} pinned", pins.len()),
            Err(e) => println!("[warn] pins: query failed ({e})"),
        }
    }

    for (bin, arg) in [("git", "--version"), ("tmux", "-V")] {
        match std::process::Command::new(bin).arg(arg).output() {
            Ok(o) if o.status.success() => {
                let first = String::from_utf8_lossy(&o.stdout);
                println!(
                    "[ok]   {}: {}",
                    bin,
                    first.lines().next().unwrap_or("").trim()
                );
            }
            _ => println!("[warn] {bin}: not found (needed from M1/M4 onward)"),
        }
    }

    // LLM key presence checks
    for (prov, provider) in [
        ("openai", rat_brain::backend::Provider::OpenAi),
        ("anthropic", rat_brain::backend::Provider::Anthropic),
        ("openrouter", rat_brain::backend::Provider::OpenRouter),
    ] {
        if rat_brain::keys::key_present(provider) {
            println!("[ok]   key {}: present", prov);
        } else {
            println!("[warn] key {}: not found (run `rat setup` to import)", prov);
        }
    }

    Ok(())
}

fn count_ring_segments(ring_dir: &Path) -> usize {
    ["screen", "audio", "clipboard"]
        .iter()
        .map(|media| {
            std::fs::read_dir(ring_dir.join(media))
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("seg"))
                        .count()
                })
                .unwrap_or(0)
        })
        .sum()
}
