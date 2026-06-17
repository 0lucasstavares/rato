use rat_brain::detect::{error_burst, stuck_loop, Signal};
use rat_proto::Observation;
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
    assert!(sig.is_some(), "expected StuckLoop signal");
    if let Some(Signal::StuckLoop {
        cmd,
        count,
        obs_ids,
    }) = sig
    {
        assert_eq!(cmd, "cargo test");
        assert_eq!(count, 3);
        assert_eq!(obs_ids.len(), 3);
        let mut sorted = obs_ids.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["a", "b", "c"]);
    } else {
        panic!("expected StuckLoop variant");
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
    assert!(
        stuck_loop(&obs).is_none(),
        "spread > 10 min should not trigger stuck_loop"
    );
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
        panic!("expected ErrorBurst variant");
    }
}

#[test]
fn zero_exit_not_counted() {
    // Mix of success and failure; only 2 failures → no signal
    let obs = vec![
        make_obs("a", 0, "shell_cmd", "cargo test", 0), // success
        make_obs("b", 100_000, "shell_cmd", "cargo test", 1),
        make_obs("c", 200_000, "shell_cmd", "cargo test", 0), // success
        make_obs("d", 300_000, "shell_cmd", "cargo test", 1),
    ];
    assert!(stuck_loop(&obs).is_none());
}

#[test]
fn different_commands_no_stuck_loop() {
    // Each command is different → no group of 3 same commands
    let obs = vec![
        make_obs("a", 0, "shell_cmd", "cmd1", 1),
        make_obs("b", 100_000, "shell_cmd", "cmd2", 1),
        make_obs("c", 200_000, "shell_cmd", "cmd3", 1),
    ];
    assert!(stuck_loop(&obs).is_none());
}

#[test]
fn exactly_ten_fails_in_window_triggers_burst() {
    let obs: Vec<Observation> = (0..10)
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
    assert!(error_burst(&obs).is_some());
}

#[test]
fn nine_fails_no_burst() {
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
fn ten_fails_outside_window_no_burst() {
    // 10 fails but spread over more than 5 min (> 300_000 ms)
    let obs: Vec<Observation> = (0..10)
        .map(|i| {
            make_obs(
                &format!("id{i}"),
                (i as i64) * 40_000, // 40s apart → total = 360_000 ms > 300_000
                "shell_cmd",
                &format!("cmd{i}"),
                1,
            )
        })
        .collect();
    assert!(error_burst(&obs).is_none());
}
