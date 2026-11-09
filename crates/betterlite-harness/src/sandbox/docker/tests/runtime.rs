use std::fs;
use std::time::Duration;

use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::timeout;

use super::{
    command, docker_script, docker_script_with, invocations, invocations_of, sandbox, tempdir,
};
use crate::sandbox::Process as _;
use crate::sandbox::Sandbox as _;
use crate::sandbox::SandboxCommand;

#[tokio::test]
async fn start_runs_one_container_and_returns_the_trimmed_id() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 0");
    let sandbox = sandbox(program);
    assert_eq!(sandbox.start().await.unwrap(), "container-1");
    assert_eq!(sandbox.start().await.unwrap(), "container-1");
    let calls = invocations(&dir);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls.first(), Some(&sandbox.run_args()));
}

#[tokio::test]
async fn start_serializes_concurrent_callers_on_one_container() {
    let dir = tempdir();
    let program = docker_script_with(
        &dir,
        "sleep 0.3\nprintf 'container-1\\n'",
        "exit 0",
        "exit 0",
    );
    let sandbox = sandbox(program);
    let (first, second) = tokio::join!(sandbox.start(), sandbox.start());
    assert_eq!(first.unwrap(), "container-1");
    assert_eq!(second.unwrap(), "container-1");
    let calls = invocations(&dir);
    assert_eq!(invocations_of(&calls, "run").len(), 1);
}

#[tokio::test]
async fn start_reports_a_failed_docker_run() {
    let dir = tempdir();
    let program = docker_script_with(
        &dir,
        "printf 'no daemon\\n' >&2\nexit 17",
        "exit 0",
        "exit 0",
    );
    let sandbox = sandbox(program);
    let error = sandbox.start().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: docker run test-image failed (17): no daemon"
    );
}

#[tokio::test]
async fn start_rejects_a_blank_container_id() {
    let dir = tempdir();
    let program = docker_script_with(&dir, "printf '   \\n'", "exit 0", "exit 0");
    let sandbox = sandbox(program);
    let error = sandbox.start().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: docker run returned no container id"
    );
}

#[tokio::test]
async fn start_replaces_invalid_utf8_in_the_container_id() {
    let dir = tempdir();
    let program = docker_script_with(&dir, "printf 'cont\\377iner\\n'", "exit 0", "exit 0");
    let sandbox = sandbox(program);
    assert_eq!(sandbox.start().await.unwrap(), "cont\u{fffd}iner");
}

#[tokio::test]
async fn start_refuses_after_close() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 0");
    let sandbox = sandbox(program);
    sandbox.close().await.unwrap();
    let error = sandbox.start().await.unwrap_err();
    assert_eq!(error.to_string(), "the sandbox failed: sandbox is closed");
    assert!(invocations(&dir).is_empty());
}

#[tokio::test]
async fn exec_rejects_an_empty_command_before_starting_docker() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 0");
    let sandbox = sandbox(program);
    let Err(error) = sandbox.exec(&SandboxCommand::default()).await else {
        panic!("an empty command must be rejected");
    };
    assert_eq!(
        error.to_string(),
        "the sandbox failed: exec() needs a non-empty command"
    );
    assert!(invocations(&dir).is_empty());
}

#[tokio::test]
async fn exec_runs_the_container_command_with_env_and_cwd() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'ready'");
    let sandbox = sandbox(program);
    let mut request = command(&["printenv", "A"]);
    request.env.insert("A".to_owned(), SecretString::from("1"));
    request.cwd = Some("sub".to_owned());
    let mut client = sandbox.exec(&request).await.unwrap();
    let mut output = String::new();
    client.stdout().read_to_string(&mut output).await.unwrap();
    assert_eq!(output, "ready");
    assert_eq!(client.wait().await.unwrap(), 0_i32);
    let calls = invocations(&dir);
    assert_eq!(
        calls.get(1),
        Some(&vec![
            "exec".to_owned(),
            "-i".to_owned(),
            "-w".to_owned(),
            "/workspace/sub".to_owned(),
            "-e".to_owned(),
            "A=1".to_owned(),
            "container-1".to_owned(),
            "printenv".to_owned(),
            "A".to_owned(),
        ])
    );
}

#[tokio::test]
async fn exec_pipes_stdin_and_closes_it() {
    let dir = tempdir();
    let program = docker_script(&dir, "cat");
    let sandbox = sandbox(program);
    let mut client = sandbox.exec(&command(&["cat"])).await.unwrap();
    client.stdin().unwrap().write_all(b"hello").await.unwrap();
    client.close_stdin();
    let mut output = Vec::new();
    client.stdout().read_to_end(&mut output).await.unwrap();
    assert_eq!(output, b"hello");
    assert_eq!(client.wait().await.unwrap(), 0_i32);
}

#[tokio::test]
async fn docker_clients_inherit_the_host_environment() {
    let dir = tempdir();
    // a shell invents a PATH for itself but does not export it, so env sees one only when inherited
    let program = docker_script(
        &dir,
        "if env | grep -q '^PATH='; then printf 'inherited'; else printf 'cleared'; fi",
    );
    let sandbox = sandbox(program);
    let output = sandbox
        .run(&command(&["env"]), Duration::from_secs(2), 1024)
        .await
        .unwrap();
    assert_eq!(output.stdout, "inherited");
}

#[tokio::test]
async fn exec_leaves_the_exit_code_to_the_caller() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 7");
    let sandbox = sandbox(program);
    let mut client = sandbox.exec(&command(&["false"])).await.unwrap();
    assert_eq!(client.wait().await.unwrap(), 7_i32);
}

#[tokio::test]
async fn run_returns_decoded_output_and_the_exit_code() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf 'ok\\377'; printf 'bad\\376' >&2\nexit 3");
    let sandbox = sandbox(program);
    let output = sandbox
        .run(&command(&["sh"]), Duration::from_secs(2), 1024)
        .await
        .unwrap();
    assert_eq!(output.stdout, "ok\u{fffd}");
    assert_eq!(output.stderr, "bad\u{fffd}");
    assert_eq!(output.exit_code, 3_i32);
    assert!(sandbox.state.lock().await.processes.is_empty());
}

#[tokio::test]
async fn run_closes_stdin_before_reading_output() {
    let dir = tempdir();
    let program = docker_script(&dir, "cat");
    let sandbox = sandbox(program);
    let output = sandbox
        .run(&command(&["cat"]), Duration::from_secs(2), 1024)
        .await
        .unwrap();
    assert_eq!(output.stdout, "");
    assert_eq!(output.exit_code, 0_i32);
}

#[tokio::test]
async fn run_times_out_and_kills_the_docker_client() {
    let dir = tempdir();
    let program = docker_script(
        &dir,
        "trap 'printf killed > \"$dir/killed\"' TERM\nsleep 30",
    );
    let sandbox = sandbox(program);
    let error = sandbox
        .run(&command(&["sleep", "30"]), Duration::from_secs(1), 1024)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the sandbox failed: command timed out after 1s: sleep"
    );
    assert!(sandbox.state.lock().await.processes.is_empty());
    let marker = fs::read_to_string(dir.path().join("killed")).unwrap();
    assert_eq!(marker, "killed");
}

#[tokio::test]
async fn run_accepts_output_at_the_byte_cap() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '0123456789'");
    let sandbox = sandbox(program);
    let output = sandbox
        .run(&command(&["echo"]), Duration::from_secs(2), 10)
        .await
        .unwrap();
    assert_eq!(output.stdout, "0123456789");
}

#[tokio::test]
async fn run_caps_its_output_and_kills_the_client() {
    let dir = tempdir();
    let program = docker_script(&dir, "printf '0123456789'");
    let sandbox = sandbox(program);
    let error = sandbox
        .run(&command(&["echo"]), Duration::from_secs(2), 4)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "sandbox run output exceeded the 4-byte content limit"
    );
    assert!(sandbox.state.lock().await.processes.is_empty());
}

#[tokio::test]
async fn close_kills_registered_processes() {
    let dir = tempdir();
    let program = docker_script(&dir, "exec sleep 30");
    let sandbox = sandbox(program);
    let mut first = sandbox.exec(&command(&["sleep", "30"])).await.unwrap();
    let mut second = sandbox.exec(&command(&["sleep", "30"])).await.unwrap();
    assert_eq!(sandbox.state.lock().await.processes.len(), 2);
    sandbox.close().await.unwrap();
    assert!(sandbox.state.lock().await.processes.is_empty());
    assert_eq!(first.wait().await.unwrap(), -15_i32);
    assert_eq!(second.wait().await.unwrap(), -15_i32);
    sandbox.close().await.unwrap();
}

#[tokio::test]
async fn close_removes_the_container_once() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 0");
    let sandbox = sandbox(program);
    sandbox.start().await.unwrap();
    sandbox.close().await.unwrap();
    sandbox.close().await.unwrap();
    let calls = invocations(&dir);
    let removals = invocations_of(&calls, "rm");
    assert_eq!(removals.len(), 1);
    assert_eq!(
        removals.first(),
        Some(&vec![
            "rm".to_owned(),
            "-f".to_owned(),
            "container-1".to_owned(),
        ])
    );
}

#[tokio::test]
async fn close_ignores_a_failed_docker_remove() {
    let dir = tempdir();
    let program = docker_script_with(
        &dir,
        "printf 'container-1\\n'",
        "exit 0",
        "printf 'gone\\n' >&2\nexit 9",
    );
    let sandbox = sandbox(program);
    sandbox.start().await.unwrap();
    sandbox.close().await.unwrap();
}

#[tokio::test]
async fn close_reports_a_missing_docker_program() {
    let dir = tempdir();
    let program = docker_script(&dir, "exit 0");
    let sandbox = sandbox(program.clone());
    sandbox.start().await.unwrap();
    fs::remove_file(&program).unwrap();
    let error = sandbox.close().await.unwrap_err();
    assert!(
        error.to_string().contains("could not run docker"),
        "unexpected message: {error}"
    );
}

#[tokio::test]
async fn dropping_a_process_handle_kills_the_docker_client() {
    let dir = tempdir();
    let program = docker_script(&dir, "exec sleep 30");
    let sandbox = sandbox(program);
    let client = sandbox.exec(&command(&["sleep", "30"])).await.unwrap();
    drop(client);
    let registered = sandbox
        .state
        .lock()
        .await
        .processes
        .first()
        .cloned()
        .unwrap();
    let status = timeout(Duration::from_secs(2), registered.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, -9_i32);
}

#[tokio::test]
async fn drop_kills_the_owned_docker_client() {
    let dir = tempdir();
    let program = docker_script(&dir, "exec sleep 30");
    let sandbox = sandbox(program);
    let mut client = sandbox.exec(&command(&["sleep", "30"])).await.unwrap();
    drop(sandbox);
    let status = timeout(Duration::from_secs(2), client.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, -9_i32);
}
