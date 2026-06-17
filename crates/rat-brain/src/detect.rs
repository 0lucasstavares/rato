use rat_proto::Observation;

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    StuckLoop {
        cmd: String,
        count: usize,
        obs_ids: Vec<String>,
    },
    ErrorBurst {
        obs_ids: Vec<String>,
    },
}

fn exit_code(obs: &Observation) -> i64 {
    obs.meta.get("exit").and_then(|v| v.as_i64()).unwrap_or(0)
}

fn is_env_assignment(token: &str) -> bool {
    if let Some(eq_pos) = token.find('=') {
        let key = &token[..eq_pos];
        !key.is_empty()
            && key
                .chars()
                .next()
                .map(|c| c.is_uppercase() || c == '_')
                .unwrap_or(false)
            && key
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    } else {
        false
    }
}

fn strip_env_assignments(s: &str) -> &str {
    let mut rest = s;
    loop {
        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..token_end];
        if is_env_assignment(token) {
            rest = rest[token_end..].trim_start();
            if rest.is_empty() {
                break;
            }
        } else {
            break;
        }
    }
    rest
}

fn normalize_cmd(content: &str) -> String {
    let trimmed = content.trim();
    let without_env = strip_env_assignments(trimmed);
    without_env.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn stuck_loop(obs: &[Observation]) -> Option<Signal> {
    use std::collections::HashMap;
    let mut by_cmd: HashMap<String, Vec<(i64, String)>> = HashMap::new();

    for o in obs {
        if o.kind == "shell_cmd" && exit_code(o) != 0 {
            let norm = normalize_cmd(&o.content);
            by_cmd.entry(norm).or_default().push((o.ts, o.id.clone()));
        }
    }

    for (cmd, mut entries) in by_cmd {
        entries.sort_by_key(|(ts, _)| *ts);
        if entries.len() >= 3 {
            for i in 0..=(entries.len() - 3) {
                let window_ts_min = entries[i].0;
                let window_ts_max = entries[i + 2].0;
                if window_ts_max - window_ts_min <= 600_000 {
                    let count = entries.len();
                    let obs_ids = entries[i..i + 3].iter().map(|(_, id)| id.clone()).collect();
                    return Some(Signal::StuckLoop {
                        cmd,
                        count,
                        obs_ids,
                    });
                }
            }
        }
    }
    None
}

pub fn error_burst(obs: &[Observation]) -> Option<Signal> {
    let mut fails: Vec<(i64, String)> = obs
        .iter()
        .filter(|o| o.kind == "shell_cmd" && exit_code(o) != 0)
        .map(|o| (o.ts, o.id.clone()))
        .collect();
    fails.sort_by_key(|(ts, _)| *ts);

    if fails.len() >= 10 {
        for i in 0..=(fails.len() - 10) {
            if fails[i + 9].0 - fails[i].0 <= 300_000 {
                let obs_ids = fails[i..i + 10].iter().map(|(_, id)| id.clone()).collect();
                return Some(Signal::ErrorBurst { obs_ids });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_obs(id: &str, ts: i64, kind: &str, content: &str, exit: i64) -> Observation {
        Observation {
            id: id.to_string(),
            event_id: None,
            ts,
            kind: kind.to_string(),
            project_id: None,
            content: content.to_string(),
            meta: json!({"exit": exit}),
        }
    }

    #[test]
    fn two_fails_no_signal() {
        let obs = vec![
            make_obs("a", 0, "shell_cmd", "cargo test", 1),
            make_obs("b", 100_000, "shell_cmd", "cargo test", 1),
        ];
        assert!(stuck_loop(&obs).is_none());
    }

    #[test]
    fn three_within_window() {
        let obs = vec![
            make_obs("a", 0, "shell_cmd", "cargo test", 1),
            make_obs("b", 200_000, "shell_cmd", "cargo test", 1),
            make_obs("c", 400_000, "shell_cmd", "cargo test", 1),
        ];
        let sig = stuck_loop(&obs);
        assert!(sig.is_some());
        if let Some(Signal::StuckLoop {
            cmd,
            count,
            obs_ids,
        }) = sig
        {
            assert_eq!(cmd, "cargo test");
            assert_eq!(count, 3);
            assert_eq!(obs_ids.len(), 3);
            assert!(obs_ids.contains(&"a".to_string()));
            assert!(obs_ids.contains(&"b".to_string()));
            assert!(obs_ids.contains(&"c".to_string()));
        } else {
            panic!("expected StuckLoop");
        }
    }

    #[test]
    fn three_spread_over_11_min() {
        // 0, 300_000, 660_001 → spread is 660_001 ms > 600_000
        let obs = vec![
            make_obs("a", 0, "shell_cmd", "cargo test", 1),
            make_obs("b", 300_000, "shell_cmd", "cargo test", 1),
            make_obs("c", 660_001, "shell_cmd", "cargo test", 1),
        ];
        assert!(stuck_loop(&obs).is_none());
    }

    #[test]
    fn ten_mixed_commands_five_min() {
        // 10 different commands all failing within 300_000 ms
        let cmds = [
            "cmd1", "cmd2", "cmd3", "cmd4", "cmd5", "cmd6", "cmd7", "cmd8", "cmd9", "cmd10",
        ];
        let obs: Vec<Observation> = cmds
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                make_obs(
                    &format!("id{i}"),
                    (i as i64) * 30_000, // 30s apart, total span = 270_000 ms
                    "shell_cmd",
                    cmd,
                    1,
                )
            })
            .collect();
        let sig = error_burst(&obs);
        assert!(sig.is_some(), "expected ErrorBurst signal");
        if let Some(Signal::ErrorBurst { obs_ids }) = sig {
            assert_eq!(obs_ids.len(), 10);
        } else {
            panic!("expected ErrorBurst");
        }
    }

    #[test]
    fn error_burst_nine_fails_no_signal() {
        let obs: Vec<Observation> = (0..9)
            .map(|i| {
                make_obs(
                    &format!("id{i}"),
                    (i as i64) * 1_000,
                    "shell_cmd",
                    &format!("cmd{i}"),
                    1,
                )
            })
            .collect();
        assert!(error_burst(&obs).is_none());
    }

    #[test]
    fn normalize_cmd_strips_env() {
        assert_eq!(normalize_cmd("FOO=bar cargo test"), "cargo test");
        assert_eq!(normalize_cmd("FOO=bar BAZ=qux  cmd  arg"), "cmd arg");
        assert_eq!(normalize_cmd("  cargo   build  "), "cargo build");
    }

    #[test]
    fn env_cmd_treated_as_same_loop() {
        // "FOO=1 make build" and "FOO=2 make build" should normalize to same
        let obs = vec![
            make_obs("a", 0, "shell_cmd", "FOO=1 make build", 1),
            make_obs("b", 100_000, "shell_cmd", "FOO=2 make build", 1),
            make_obs("c", 200_000, "shell_cmd", "BAR=xyz make build", 1),
        ];
        let sig = stuck_loop(&obs);
        assert!(
            sig.is_some(),
            "env-prefixed commands should normalize to same cmd"
        );
    }
}
