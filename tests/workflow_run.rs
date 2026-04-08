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
            d.join("04_workflow.toml").to_str().unwrap(),
        ])
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
}
