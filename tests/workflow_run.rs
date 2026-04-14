use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
struct DockerSshTestEnv {
    root: PathBuf,
}

#[cfg(unix)]
impl DockerSshTestEnv {
    fn start(root: PathBuf) -> Self {
        let output = root.join("target/graph_run_docker_ssh_it_constants.toml");
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).expect("create target/ for docker IT constants");
        }
        let up = root.join("scripts/docker-ssh-test-up.sh");
        let status = Command::new("bash")
            .arg(&up)
            .current_dir(&root)
            .env("OUTPUT", &output)
            .status()
            .unwrap_or_else(|e| panic!("failed to spawn bash {}: {e}", up.display()));
        if !status.success() {
            Self::teardown(&root);
            panic!(
                "docker-ssh-test-up.sh failed with {status}; is Docker running and reachable?"
            );
        }
        Self { root }
    }

    fn teardown(root: &Path) {
        let down = root.join("scripts/docker-ssh-test-down.sh");
        let _ = Command::new("bash")
            .arg(&down)
            .current_dir(root)
            .status();
    }
}

#[cfg(unix)]
impl Drop for DockerSshTestEnv {
    fn drop(&mut self) {
        Self::teardown(&self.root);
    }
}

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

/// `retry` on [[tasks]]: extra runs after failure before the failure edge (bash fixture).
#[cfg(unix)]
#[test]
fn task_retry_succeeds_on_second_attempt() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data/task_retry");
    let ws = root.join("target/graph_run_task_retry_ws_ok");
    let _ = fs::remove_dir_all(&ws);
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .arg("--workspace")
        .arg(ws.to_str().unwrap())
        .args(graph_run_std_args(&d, "04_workflow_retry_success.toml"))
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");
}

#[test]
fn task_transfer_retry_exhausts_then_abort() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data/task_retry_transfer");
    let ws = root.join("target/graph_run_task_retry_transfer_ws");
    let _ = fs::remove_dir_all(&ws);
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let output = Command::new(bin)
        .arg("-v")
        .arg("--workspace")
        .arg(ws.to_str().unwrap())
        .args(graph_run_std_args(&d, "04_workflow.toml"))
        .output()
        .expect("spawn graph_run");
    assert!(!output.status.success(), "expected abort after transfer retries");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("retrying").count(),
        1,
        "expected one retry warning for transfer retry=1: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn task_retry_exhausted_then_failure_branch() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/data/task_retry");
    let ws = root.join("target/graph_run_task_retry_ws_fail");
    let _ = fs::remove_dir_all(&ws);
    let bin = env!("CARGO_BIN_EXE_graph_run");
    let output = Command::new(bin)
        .arg("-v")
        .arg("--workspace")
        .arg(ws.to_str().unwrap())
        .args(graph_run_std_args(&d, "04_workflow_retry_exhaust.toml"))
        .output()
        .expect("spawn graph_run");
    assert!(!output.status.success(), "expected abort after retries");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("retrying").count(),
        2,
        "expected two retry warnings for retry=2 (three attempts): {stderr}"
    );
    assert!(
        stderr.contains("abort") || stderr.contains("failure"),
        "stderr should mention failure path: {stderr}"
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

/// Same invocation shape as a manual run from `tests/data/test_file_transfer/`:
/// `graph_run --constants 00_constants.toml --configs 1*.toml 25_workflow.toml --workspace ./.workspace -vvvvv`
/// (here `1*.toml` is expanded in sorted order like the shell).
///
/// Starts **`scripts/docker-ssh-test-up.sh`** before `graph_run` and runs **`docker-ssh-test-down.sh`**
/// on teardown (including after failures). Requires **Docker**, **bash**, and a free **host port
/// 2222** (defaults match `tests/data/test_file_transfer/00_constants.toml`). Unix only (remote SSH
/// is not built for other targets).
#[cfg(unix)]
#[test]
fn test_file_transfer_cli_style_constants_and_globbed_configs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let _docker = DockerSshTestEnv::start(root.clone());
    let d = root.join("tests/data/test_file_transfer");
    let _ = fs::remove_dir_all(d.join(".workspace"));

    let mut config_names: Vec<String> = fs::read_dir(&d)
        .expect("read_dir test_file_transfer")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let s = e.file_name().to_string_lossy().into_owned();
            if s.starts_with('1') && s.ends_with(".toml") {
                Some(s)
            } else {
                None
            }
        })
        .collect();
    config_names.sort();

    let bin = env!("CARGO_BIN_EXE_graph_run");
    let status = Command::new(bin)
        .current_dir(&d)
        .arg("--constants")
        .arg("00_constants.toml")
        .arg("--configs")
        .args(&config_names)
        .arg("25_workflow.toml")
        .arg("--workspace")
        .arg("./.workspace")
        .arg("-vvvvv")
        .status()
        .expect("spawn graph_run");
    assert!(status.success(), "graph_run failed: {status}");

    // source_path is "$HOME/tmp" without a trailing slash: the `tmp` directory is mirrored under
    // dest, so `tmp/hi.txt` on the remote becomes `.workspace/hi.txt` (see transfer trailing-slash rules).
    let hi = d.join(".workspace/hi.txt");
    assert!(
        hi.is_file(),
        "expected SFTP pull to leave remote tmp/hi.txt as workspace/hi.txt: {}",
        hi.display()
    );
    let hello = d.join(".workspace/tmp/hello.txt");
    assert!(
        hello.is_file(),
        "expected nested remote tmp/tmp/hello.txt at workspace/tmp/hello.txt: {}",
        hello.display()
    );
}
