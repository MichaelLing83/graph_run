use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn example_workflow_exits_zero() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .args([
            "--servers",
            d.join("00_servers.toml").to_str().unwrap(),
            "--shells",
            d.join("01_shells.toml").to_str().unwrap(),
            "--commands",
            d.join("02_commands.toml").to_str().unwrap(),
            "--tasks",
            d.join("03_tasks.toml").to_str().unwrap(),
            d.join("04_workflow_linear.toml").to_str().unwrap(),
        ])
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
}

#[test]
fn workspace_creates_logs_and_tmp() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let ws = root.join("target/graph_run_it_workspace");
    let _ = fs::remove_dir_all(&ws);
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .args([
            "--servers",
            d.join("00_servers.toml").to_str().unwrap(),
            "--shells",
            d.join("01_shells.toml").to_str().unwrap(),
            "--commands",
            d.join("02_commands.toml").to_str().unwrap(),
            "--tasks",
            d.join("03_tasks.toml").to_str().unwrap(),
            "--workspace",
            ws.to_str().unwrap(),
            d.join("04_workflow_linear.toml").to_str().unwrap(),
        ])
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
    assert!(ws.join("tmp").is_dir(), "tmp/ missing");
    let logs = ws.join("logs");
    assert!(logs.is_dir(), "logs/ missing");
    let count = fs::read_dir(&logs).expect("read logs").count();
    assert!(count >= 1, "expected at least one log file");
}

#[test]
fn cyclic_workflow_rejected_without_allow_flag() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let output = Command::new(bin)
        .args([
            "--servers",
            d.join("00_servers.toml").to_str().unwrap(),
            "--shells",
            d.join("01_shells.toml").to_str().unwrap(),
            "--commands",
            d.join("02_commands.toml").to_str().unwrap(),
            "--tasks",
            d.join("03_tasks.toml").to_str().unwrap(),
            d.join("04_workflow.toml").to_str().unwrap(),
        ])
        .output()
        .expect("spawn graph_run");
    assert!(
        !output.status.success(),
        "expected failure for cyclic workflow without --allow-endless-loop"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("directed cycle") && stderr.contains("--allow-endless-loop"),
        "stderr should explain cycle and flag: {stderr}"
    );
}
