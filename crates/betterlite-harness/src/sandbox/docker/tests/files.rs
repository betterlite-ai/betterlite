use std::fs;

use super::{docker_script, invocations, sandbox, tempdir};
use crate::sandbox::Sandbox as _;
use crate::sandbox::docker::DockerSandbox;

const FIND_SCRIPT: &str = r#"cd "$1" && find . -type d \( -name '.git' -o -name '.mypy_cache' -o -name '.pytest_cache' -o -name '.ruff_cache' -o -name '.venv' -o -name '__pycache__' -o -name 'node_modules' -o -name 'venv' \) -prune -o -type f -size -52428801c -exec sha256sum {} +"#;

fn exec_call(command: &[&str]) -> Vec<String> {
    let mut args = vec![
        "exec".to_owned(),
        "-i".to_owned(),
        "-w".to_owned(),
        "/workspace".to_owned(),
        "container-1".to_owned(),
    ];
    args.extend(command.iter().map(|part| (*part).to_owned()));
    args
}

#[tokio::test]
async fn read_runs_cat_on_the_container_path() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'file body'");
    let sandbox = sandbox(program);
    let data = sandbox.read("nested/../file.txt", 100).await.unwrap();
    assert_eq!(data, b"file body");
    let calls = invocations(&dir);
    assert_eq!(
        calls.get(1),
        Some(&exec_call(&["cat", "/workspace/file.txt"]))
    );
}

#[tokio::test]
async fn read_keeps_absolute_container_paths() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'hosts'");
    let sandbox = sandbox(program);
    assert_eq!(sandbox.read("/etc/hosts", 100).await.unwrap(), b"hosts");
    let calls = invocations(&dir);
    assert_eq!(
        calls.get(1),
        Some(&vec![
            "exec".to_owned(),
            "-i".to_owned(),
            "-w".to_owned(),
            "/workspace".to_owned(),
            "container-1".to_owned(),
            "cat".to_owned(),
            "/etc/hosts".to_owned(),
        ])
    );
}

#[tokio::test]
async fn read_reports_the_target_and_stderr() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'No such file\\n' >&2\nexit 1");
    let sandbox = sandbox(program);
    let error = sandbox.read("missing", 100).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: could not read /workspace/missing: No such file"
    );
}

#[tokio::test]
async fn read_accepts_output_at_the_byte_cap() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '12345'");
    let sandbox = sandbox(program);
    assert_eq!(sandbox.read("file", 5).await.unwrap(), b"12345");
}

#[tokio::test]
async fn read_enforces_the_byte_cap() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '12345'");
    let sandbox = sandbox(program);
    let error = sandbox.read("file", 4).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "sandbox read exceeded the 4-byte content limit"
    );
}

#[tokio::test]
async fn read_caps_container_output_at_the_helper_limit() {
    let dir = tempdir();
    let program = docker_script(&dir, "head -c 67108865 /dev/zero");
    let sandbox = sandbox(program);
    let error = sandbox.read("huge", 1).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "sandbox read exceeded the 67108864-byte content limit"
    );
}

#[tokio::test]
async fn write_streams_data_through_the_write_script() {
    let dir = tempdir();
    let program = docker_script(&dir, "cat > \"$dir/written\"");
    let sandbox = sandbox(program);
    sandbox.write("nested/file.bin", b"payload").await.unwrap();
    assert_eq!(fs::read(dir.path().join("written")).unwrap(), b"payload");
    let calls = invocations(&dir);
    assert_eq!(
        calls.get(1),
        Some(&vec![
            "exec".to_owned(),
            "-i".to_owned(),
            "-w".to_owned(),
            "/workspace".to_owned(),
            "container-1".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            "mkdir -p \"$(dirname \"$1\")\" && cat > \"$1\"".to_owned(),
            "sh".to_owned(),
            "/workspace/nested/file.bin".to_owned(),
        ])
    );
}

#[tokio::test]
async fn write_reports_the_target_and_stderr() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'denied\\n' >&2\nexit 2");
    let sandbox = sandbox(program);
    let error = sandbox.write("x", b"data").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: could not write /workspace/x: denied"
    );
}

#[tokio::test]
async fn write_keeps_the_container_error_when_docker_exits_before_reading_stdin() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'no space left\\n' >&2\nexit 2");
    let sandbox = sandbox(program);
    let payload = vec![b'x'; 0x0002_0000];
    let error = sandbox.write("big", &payload).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: could not write /workspace/big: no space left"
    );
}

#[tokio::test]
async fn which_returns_the_trimmed_path_on_success() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '  /usr/bin/python3  \\n'");
    let sandbox = sandbox(program);
    assert_eq!(
        sandbox.which("python3").await.unwrap(),
        Some("/usr/bin/python3".to_owned())
    );
    let calls = invocations(&dir);
    assert_eq!(
        calls.get(1),
        Some(&vec![
            "exec".to_owned(),
            "-i".to_owned(),
            "-w".to_owned(),
            "/workspace".to_owned(),
            "container-1".to_owned(),
            "sh".to_owned(),
            "-lc".to_owned(),
            "command -v \"$1\"".to_owned(),
            "sh".to_owned(),
            "python3".to_owned(),
        ])
    );
}

#[tokio::test]
async fn which_returns_none_for_a_failed_lookup() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '/usr/bin/x'\nexit 1");
    let sandbox = sandbox(program);
    assert_eq!(sandbox.which("x").await.unwrap(), None);
}

#[tokio::test]
async fn which_returns_none_for_blank_output() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '   \\n'");
    let sandbox = sandbox(program);
    assert_eq!(sandbox.which("x").await.unwrap(), None);
}

#[tokio::test]
async fn tempdir_returns_the_trimmed_mktemp_path() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '  /tmp/tmp.abc  \\n'");
    let sandbox = sandbox(program);
    assert_eq!(sandbox.tempdir().await.unwrap(), "/tmp/tmp.abc");
    let calls = invocations(&dir);
    assert_eq!(
        calls.get(1),
        Some(&vec![
            "exec".to_owned(),
            "-i".to_owned(),
            "-w".to_owned(),
            "/workspace".to_owned(),
            "container-1".to_owned(),
            "mktemp".to_owned(),
            "-d".to_owned(),
        ])
    );
}

#[tokio::test]
async fn tempdir_reports_a_failed_mktemp() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'no space\\n' >&2\nexit 1");
    let sandbox = sandbox(program);
    let error = sandbox.tempdir().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: mktemp -d failed: no space"
    );
}

#[tokio::test]
async fn tempdir_reports_blank_output() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 0");
    let sandbox = sandbox(program);
    let error = sandbox.tempdir().await.unwrap_err();
    assert_eq!(error.to_string(), "the sandbox failed: mktemp -d failed: ");
}

#[tokio::test]
async fn snapshot_runs_the_sorted_find_script() {
    let dir = tempdir();
    let first = format!(
        "{}  ./src/main.rs\n{}  lib.rs\n",
        "a".repeat(64),
        "b".repeat(64)
    );
    let program = docker_script(&dir, &format!("cat <<'EOF'\n{first}EOF"));
    let sandbox = sandbox(program);
    let snapshot = sandbox.snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot.get("src/main.rs"), Some(&"a".repeat(64)));
    assert_eq!(snapshot.get("lib.rs"), Some(&"b".repeat(64)));
    let calls = invocations(&dir);
    assert_eq!(
        calls.get(1),
        Some(&vec![
            "exec".to_owned(),
            "-i".to_owned(),
            "-w".to_owned(),
            "/workspace".to_owned(),
            "container-1".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            FIND_SCRIPT.to_owned(),
            "sh".to_owned(),
            "/workspace".to_owned(),
        ])
    );
}

#[tokio::test]
async fn snapshot_passes_its_own_workdir() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 0");
    let sandbox = DockerSandbox::new(
        "test-image".to_owned(),
        Vec::new(),
        Some("/work".to_owned()),
        Vec::new(),
        None,
    )
    .unwrap()
    .with_docker_program(program);
    let snapshot = sandbox.snapshot().await.unwrap();
    assert!(snapshot.is_empty());
    let calls = invocations(&dir);
    let exec = &calls[1];
    assert_eq!(exec.get(3).map(String::as_str), Some("/work"));
    assert_eq!(exec.get(9).map(String::as_str), Some("/work"));
}

#[tokio::test]
async fn snapshot_fails_with_the_docker_error_text() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'boom\\n' >&2\nexit 1");
    let sandbox = sandbox(program);
    let error = sandbox.snapshot().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: snapshot failed: boom"
    );
}
