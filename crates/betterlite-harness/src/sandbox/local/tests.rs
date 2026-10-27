#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::{Instant, timeout};

use secrecy::SecretString;

use super::LocalSandbox;
use super::environment::{filtered_environ, is_secret_env_name};
use super::files::decode_utf8;
use crate::sandbox::{Process as _, Sandbox as _, SandboxCommand};
use secrecy::ExposeSecret;

fn tempdir() -> TempDir {
    tempfile::Builder::new()
        .prefix("betterlite-local-test-")
        .tempdir()
        .unwrap()
}

#[test]
fn filters_provider_credentials_but_extra_values_win() {
    assert!(is_secret_env_name("openai_api_key"));
    assert!(is_secret_env_name("SERVICE_SECRET_VALUE"));
    assert!(!is_secret_env_name("PATH"));
    let mut extra = BTreeMap::new();
    extra.insert("OPENAI_API_KEY".to_owned(), SecretString::from("explicit"));
    assert_eq!(
        filtered_environ(&BTreeMap::new(), &extra)
            .get("OPENAI_API_KEY")
            .map(ExposeSecret::expose_secret),
        Some("explicit")
    );
}

#[tokio::test]
async fn executes_commands_and_confines_paths() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let command = SandboxCommand {
        argv: vec!["printf".to_owned(), "hello".to_owned()],
        env: BTreeMap::new(),
        cwd: None,
    };
    let run = sandbox
        .run(&command, Duration::from_secs(2), 1024)
        .await
        .unwrap();
    assert_eq!(run.stdout, "hello");
    sandbox.resolve_path("../outside").unwrap_err();
}

#[tokio::test]
async fn reads_writes_and_cleans_tempdirs() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    sandbox.write("nested/file", b"hello\xff").await.unwrap();
    assert_eq!(
        sandbox.read("nested/file", 100).await.unwrap(),
        b"hello\xff"
    );
    let temporary = sandbox.tempdir().await.unwrap();
    let temporary_path = Path::new(&temporary);
    assert!(temporary.starts_with('/'));
    assert!(temporary_path.is_dir());
    sandbox.close().await.unwrap();
    assert!(!temporary_path.exists());
}

#[tokio::test]
async fn rejects_empty_commands_and_file_cwds() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let empty = SandboxCommand::default();
    assert!(sandbox.exec(&empty).await.is_err());
    fs::write(root.path().join("file"), b"x").unwrap();
    let command = SandboxCommand {
        argv: vec!["true".to_owned()],
        cwd: Some("file".to_owned()),
        env: BTreeMap::new(),
    };
    let Err(error) = sandbox.exec(&command).await else {
        panic!("a file is not a valid cwd");
    };
    assert!(error.to_string().contains("cwd is not a directory"));
}

#[test]
fn filters_claudecode_prefix() {
    assert!(is_secret_env_name("CLAUDECODE_SESSION"));
}

#[test]
fn replaces_truncated_utf8_once() {
    assert_eq!(decode_utf8(b"ok\xe2\x82"), "ok\u{fffd}");
}

#[tokio::test]
async fn snapshot_rejects_closed() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    sandbox.close().await.unwrap();
    sandbox.snapshot().await.unwrap_err();
}

fn command(argv: &[&str]) -> SandboxCommand {
    SandboxCommand {
        argv: argv.iter().map(|part| (*part).to_owned()).collect(),
        env: BTreeMap::new(),
        cwd: None,
    }
}

#[rstest::rstest]
#[case::anthropic("ANTHROPIC_HOST")]
#[case::openai("OPENAI_HOST")]
#[case::litellm("litellm_host")]
#[case::azure("AZURE_HOST")]
#[case::aws("AWS_REGION")]
#[case::gemini("GEMINI_HOST")]
#[case::codex("CODEX_HOME")]
#[case::cursor("CURSOR_HOME")]
#[case::vertex("VERTEX_PROJECT")]
#[case::claude_code("CLAUDE_CODE_SOCKET")]
#[case::claude_pid("CLAUDE_PID_OTHER")]
#[case::claudecode("CLAUDECODE_OTHER")]
#[case::google_key("GOOGLE_API_KEY")]
#[case::google_credentials("GOOGLE_APPLICATION_CREDENTIALS")]
#[case::api_key("custom_api_key")]
#[case::token("custom_token")]
#[case::secret("custom_secret")]
#[tokio::test]
async fn child_environment_is_filtered(#[case] name: &str) {
    let root = tempdir();
    let environment = BTreeMap::from([
        (name.to_owned(), SecretString::from("must-not-escape")),
        ("VISIBLE".to_owned(), SecretString::from("kept")),
    ]);
    let sandbox = LocalSandbox::new(root.path(), &environment, root.path().to_path_buf()).unwrap();
    let output = sandbox
        .run(&command(&["/usr/bin/env"]), Duration::from_secs(2), 4096)
        .await
        .unwrap();
    assert_eq!(output.stdout, "VISIBLE=kept\n");
    let mut request = command(&["/usr/bin/env"]);
    request
        .env
        .insert(name.to_owned(), SecretString::from("override"));
    let overridden = sandbox
        .run(&request, Duration::from_secs(2), 4096)
        .await
        .unwrap();
    assert!(overridden.stdout.contains(&format!("{name}=override\n")));
}

#[tokio::test]
async fn nonzero_and_binary_output() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let output = sandbox
        .run(
            &command(&["/bin/sh", "-c", r"printf '\377'; printf 'err' >&2; exit 7"]),
            Duration::from_secs(2),
            100,
        )
        .await
        .unwrap();
    assert_eq!(output.stdout, "\u{fffd}");
    assert_eq!(output.stderr, "err");
    assert_eq!(output.exit_code, 7_i32);
    let mut process = sandbox.exec(&command(&["/bin/false"])).await.unwrap();
    assert_eq!(process.wait().await.unwrap(), 1_i32);
    process.kill().await.unwrap();
}

#[tokio::test]
async fn closes_stdin_and_caps_output() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let output = sandbox
        .run(&command(&["/bin/cat"]), Duration::from_secs(2), 100)
        .await
        .unwrap();
    assert_eq!(output.stdout, "");
    let error = sandbox
        .run(&command(&["/usr/bin/yes"]), Duration::from_secs(2), 10)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("10-byte"));
}

#[tokio::test]
async fn timeout_names_duration_and_cmd() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let error = sandbox
        .run(&command(&["/bin/sleep", "30"]), Duration::from_secs(1), 100)
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("timed out after 1s"));
    assert!(message.contains("/bin/sleep"));
}

#[tokio::test]
async fn close_kills_and_is_idempotent() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let mut child = sandbox.exec(&command(&["/bin/sleep", "30"])).await.unwrap();
    let temporary = sandbox.tempdir().await.unwrap();
    sandbox.close().await.unwrap();
    sandbox.close().await.unwrap();
    assert_eq!(child.wait().await.unwrap(), -15_i32);
    assert!(!Path::new(&temporary).exists());
    sandbox.read("file", 10).await.unwrap_err();
    sandbox.write("file", b"data").await.unwrap_err();
    sandbox.tempdir().await.unwrap_err();
}

#[tokio::test]
async fn drop_kills_owned_process() {
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let mut child = sandbox.exec(&command(&["/bin/sleep", "30"])).await.unwrap();
    drop(sandbox);
    let status = timeout(Duration::from_secs(2), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, -9_i32);
}

#[tokio::test]
async fn escalates_after_grace_period() {
    use tokio::io::AsyncReadExt as _;
    let root = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let mut child = sandbox
        .exec(&command(&[
            "/bin/sh",
            "-c",
            "trap '' TERM; printf ready; exec /bin/sleep 30",
        ]))
        .await
        .unwrap();
    let mut ready = [0_u8; 5];
    child.stdout().read_exact(&mut ready).await.unwrap();
    let started = Instant::now();
    child.kill().await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(child.wait().await.unwrap(), -9_i32);
    child.kill().await.unwrap();
}

#[tokio::test]
async fn confines_symlinks_and_parents() {
    use std::os::unix::fs::symlink;
    let root = tempdir();
    let outside = tempdir();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    symlink(outside.path().join("absent"), root.path().join("escape")).unwrap();
    let error = sandbox.write("escape/child", b"bad").await.unwrap_err();
    assert!(error.to_string().contains("escape"));
    sandbox
        .write("missing/../../outside", b"bad")
        .await
        .unwrap_err();
    assert!(!outside.path().join("absent").exists());
    sandbox.write("inside/file", b"good").await.unwrap();
    symlink("inside", root.path().join("allowed")).unwrap();
    assert_eq!(sandbox.read("allowed/file", 100).await.unwrap(), b"good");
    let temporary = sandbox.tempdir().await.unwrap();
    assert!(temporary.contains("litellm-harness-"));
    sandbox
        .write(&format!("{temporary}/file"), b"temp")
        .await
        .unwrap();
    assert_eq!(
        sandbox
            .read(&format!("{temporary}/file"), 100)
            .await
            .unwrap(),
        b"temp"
    );
}

#[tokio::test]
async fn which_uses_injected_path() {
    use std::os::unix::fs::symlink;
    let root = tempdir();
    symlink("/bin/true", root.path().join("chosen")).unwrap();
    fs::write(root.path().join("plain"), b"text").unwrap();
    let environment = BTreeMap::from([(
        "PATH".to_owned(),
        SecretString::from(root.path().to_str().unwrap()),
    )]);
    let sandbox = LocalSandbox::new(root.path(), &environment, root.path().to_path_buf()).unwrap();
    assert_eq!(
        sandbox.which("chosen").await.unwrap(),
        Some(root.path().join("chosen").to_str().unwrap().to_owned())
    );
    assert_eq!(sandbox.which("plain").await.unwrap(), None);
    assert_eq!(sandbox.which("missing").await.unwrap(), None);
    assert_eq!(sandbox.host_url(8123), "http://127.0.0.1:8123");
    sandbox.write("file", b"snapshot").await.unwrap();
    assert!(sandbox.snapshot().await.unwrap().contains_key("file"));
}
