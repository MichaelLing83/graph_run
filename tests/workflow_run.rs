use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `--configs` plus the usual split fixture files and a workflow file name under `d`.
fn graph_run_std_args(d: &Path, workflow: &str) -> Vec<String> {
    let mut v = vec!["--configs".to_string()];
    for name in [
        "00_servers.toml",
        "01_shells.toml",
        "02_commands.toml",
        "03_tasks.toml",
    ] {
        v.push(d.join(name).to_string_lossy().into_owned());
    }
    v.push(d.join(workflow).to_string_lossy().into_owned());
    v
}

#[test]
fn example_workflow_exits_zero() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .args(graph_run_std_args(&d, "04_workflow_linear.toml"))
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
        .arg("--workspace")
        .arg(ws.to_str().unwrap())
        .args(graph_run_std_args(&d, "04_workflow_linear.toml"))
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
fn loop_node_runs_body_count_times() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .arg("-vv")
        .args(graph_run_std_args(&d, "04_workflow_loop.toml"))
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
}

#[test]
fn fork_join_parallel_branches() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .args(graph_run_std_args(&d, "04_workflow_fork_join.toml"))
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
}

#[test]
fn nested_loops_complete() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .args(graph_run_std_args(&d, "04_workflow_nested_loops.toml"))
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
}

#[test]
fn abort_node_exits_nonzero() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let output = Command::new(bin)
        .args(graph_run_std_args(&d, "04_workflow_abort.toml"))
        .output()
        .expect("spawn graph_run");
    assert!(
        !output.status.success(),
        "expected nonzero exit when workflow reaches abort"
    );
    #[cfg(unix)]
    assert_eq!(
        output.status.code(),
        Some(1),
        "graph_run should use exit code 1 on workflow failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("abort") && stderr.contains("failure"),
        "stderr should mention abort / failure branch: {stderr}"
    );
}

#[test]
fn constants_substitution_expands_in_configs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data/constants_subst");
    let ws = root.join("target/graph_run_constants_it_workspace");
    let _ = fs::remove_dir_all(&ws);
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .arg("--constants")
        .arg(d.join("constants.toml").to_str().unwrap())
        .arg("--workspace")
        .arg(ws.to_str().unwrap())
        .args(graph_run_std_args(&d, "04_workflow_linear.toml"))
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
    let out = ws.join("tmp/out.txt");
    let body = fs::read_to_string(&out).expect("read tmp/out.txt");
    assert_eq!(body, "constant-ok");
}

#[test]
fn constants_unknown_placeholder_errors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data/constants_subst_bad");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let output = Command::new(bin)
        .arg("--constants")
        .arg(d.join("constants.toml").to_str().unwrap())
        .args(graph_run_std_args(&d, "04_workflow_linear.toml"))
        .output()
        .expect("spawn graph_run");
    assert!(!output.status.success(), "expected failure for unknown constant");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("UNKNOWN_CONSTANT"),
        "stderr should name missing constant: {stderr}"
    );
}

#[test]
fn cyclic_workflow_rejected_without_allow_flag() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data");
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let output = Command::new(bin)
        .args(graph_run_std_args(&d, "04_workflow.toml"))
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
