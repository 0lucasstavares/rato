use std::path::{Path, PathBuf};

use anyhow::Context;

pub fn config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").expect("HOME not set")).join(".config"),
    }
}

fn unit_contents(ratd: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=RATO daemon (ratd)\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         Environment=RAT_LOG=info\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        ratd.display()
    )
}

pub fn install(no_systemctl: bool, ratd_path: Option<PathBuf>) -> anyhow::Result<()> {
    let ratd = match ratd_path {
        Some(p) => p,
        None => std::env::current_exe()?
            .parent()
            .context("rat binary has no parent directory")?
            .join("ratd"),
    };
    anyhow::ensure!(
        ratd.exists(),
        "ratd not found at {} — build it first (cargo build --release) or pass --ratd-path",
        ratd.display()
    );

    let unit_dir = config_home().join("systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit_path = unit_dir.join("ratd.service");
    std::fs::write(&unit_path, unit_contents(&ratd))?;
    println!("wrote {}", unit_path.display());

    if no_systemctl {
        println!(
            "skipped systemctl; run: systemctl --user daemon-reload && systemctl --user enable --now ratd.service"
        );
        return Ok(());
    }
    run_systemctl(&["--user", "daemon-reload"])?;
    run_systemctl(&["--user", "enable", "--now", "ratd.service"])?;
    println!("ratd enabled and started — check: systemctl --user status ratd");
    Ok(())
}

fn run_systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()
        .with_context(|| format!("running systemctl {args:?}"))?;
    anyhow::ensure!(status.success(), "systemctl {args:?} failed");
    Ok(())
}
