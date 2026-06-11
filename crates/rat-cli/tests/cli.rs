use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

/// Run an in-process daemon on a background thread; return the socket path.
fn start_daemon(tmp: &Path) -> PathBuf {
    let socket = tmp.join("ratd.sock");
    let db = tmp.join("rato.db");
    let socket2 = socket.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let store =
                rat_store::store::Store::open(&db, Arc::new(rat_core::clock::SystemClock))
                    .unwrap();
            let ctx = Arc::new(rat_daemon::server::ServerCtx {
                store,
                started: Instant::now(),
                db_path: db,
            });
            let listener = tokio::net::UnixListener::bind(&socket2).unwrap();
            rat_daemon::server::serve(listener, ctx).await;
        });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(20));
    }
    socket
}

#[test]
fn status_emit_and_recent_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = start_daemon(tmp.path());
    let sock = socket.to_str().unwrap();

    Command::cargo_bin("rat")
        .unwrap()
        .args(["--socket", sock, "status"])
        .assert()
        .success()
        .stdout(contains("ratd").and(contains("events:")));

    Command::cargo_bin("rat")
        .unwrap()
        .args(["--socket", sock, "emit", "test_event", "--payload", r#"{"n":1}"#])
        .assert()
        .success();

    Command::cargo_bin("rat")
        .unwrap()
        .args(["--socket", sock, "events", "recent"])
        .assert()
        .success()
        .stdout(contains("test_event"));
}

#[test]
fn status_fails_cleanly_when_daemon_is_down() {
    let tmp = tempfile::tempdir().unwrap();
    let sock = tmp.path().join("nope.sock");

    Command::cargo_bin("rat")
        .unwrap()
        .args(["--socket", sock.to_str().unwrap(), "status"])
        .assert()
        .failure()
        .stderr(contains("connecting"));
}

#[test]
fn install_writes_unit_file_pointing_at_ratd() {
    let tmp = tempfile::tempdir().unwrap();
    let fake_ratd = tmp.path().join("ratd");
    std::fs::write(&fake_ratd, "#!/bin/sh\n").unwrap();
    let config = tmp.path().join("config");

    Command::cargo_bin("rat")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config)
        .args(["install", "--no-systemctl", "--ratd-path", fake_ratd.to_str().unwrap()])
        .assert()
        .success();

    let unit = config.join("systemd/user/ratd.service");
    let contents = std::fs::read_to_string(&unit).unwrap();
    assert!(contents.contains(&format!("ExecStart={}", fake_ratd.display())));
    assert!(contents.contains("Restart=on-failure"));
    assert!(contents.contains("WantedBy=default.target"));
}

#[test]
fn install_refuses_when_ratd_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");

    Command::cargo_bin("rat")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config)
        .args(["install", "--no-systemctl", "--ratd-path", "/nonexistent/ratd"])
        .assert()
        .failure()
        .stderr(contains("ratd not found"));
}

#[test]
fn doctor_reports_daemon_state_without_failing() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = start_daemon(tmp.path());

    Command::cargo_bin("rat")
        .unwrap()
        .args(["--socket", socket.to_str().unwrap(), "doctor"])
        .assert()
        .success()
        .stdout(contains("daemon").and(contains("[ok]")));
}
