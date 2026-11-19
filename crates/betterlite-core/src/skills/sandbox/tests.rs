#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::executor::{SandboxExecutor, SandboxOutcome, SandboxRequest};
use super::programs::{
    REQUIREMENTS_FILE, SANDBOX_ROOT, SESSION_LANG, SKILL_REQUIREMENTS_FILE, pip_install_code,
    wrapped_code,
};
use super::session::{
    RunResult, SandboxSession, SdkFailure, SessionError, SessionOpener, SessionOptions,
};
use crate::api::skills::hook::execution::SkillFiles;

mod decoder;
mod sdk;

const SDK_MISSING: &str = "llm-sandbox not installed. Install `llm-sandbox`.";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Reply {
    exit_code: i32,
    stdout: Option<String>,
    stderr: Option<String>,
}

fn reply(exit_code: i32, stdout: Option<&str>, stderr: Option<&str>) -> Reply {
    Reply {
        exit_code,
        stdout: stdout.map(str::to_owned),
        stderr: stderr.map(str::to_owned),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Install,
    Execution,
    Listing,
}

fn kind_of(code: &str) -> Kind {
    if code.contains("os.walk('/sandbox')") {
        Kind::Listing
    } else if code.contains("'pip', 'install', '-r'") {
        Kind::Install
    } else {
        Kind::Execution
    }
}

#[derive(Debug, Default)]
struct Calls {
    options: Option<SessionOptions>,
    copies: Vec<(String, Vec<u8>)>,
    staging: Vec<PathBuf>,
    runs: Vec<String>,
    downloaded: Vec<String>,
    closes: usize,
}

#[derive(Debug, Default)]
struct Script {
    install: Option<Reply>,
    execution: Option<Reply>,
    listing: Option<Reply>,
    runtime_files: BTreeMap<String, Vec<u8>>,
    refusing_copies: Vec<String>,
    missing_sdk: bool,
    open_error: Option<String>,
    close_error: Option<String>,
    hang: Option<Duration>,
    calls: Mutex<Calls>,
}

impl Script {
    fn calls(&self) -> MutexGuard<'_, Calls> {
        self.calls.lock().expect("script calls")
    }

    fn reply_for(&self, kind: Kind) -> Reply {
        let chosen = match kind {
            Kind::Install => &self.install,
            Kind::Execution => &self.execution,
            Kind::Listing => &self.listing,
        };
        chosen.clone().unwrap_or_default()
    }
}

struct ScriptedSession {
    script: Arc<Script>,
    closed: AtomicBool,
}

fn call_error(operation: &'static str, message: &str) -> SessionError {
    SessionError::Call {
        operation,
        source: SdkFailure::from(message.to_owned()),
    }
}

#[async_trait]
impl SandboxSession for ScriptedSession {
    async fn copy_to_runtime(&self, local: &Path, sandbox_path: &str) -> Result<(), SessionError> {
        let content = fs::read(local).expect("the staged file");
        let mut calls = self.script.calls();
        calls.staging.extend(local.parent().map(Path::to_path_buf));
        calls.copies.push((sandbox_path.to_owned(), content));
        Ok(())
    }

    async fn copy_from_runtime(
        &self,
        source: &str,
        destination: &Path,
    ) -> Result<(), SessionError> {
        let script = &self.script;
        let mut calls = script.calls();
        calls.downloaded.push(source.to_owned());
        if script.refusing_copies.iter().any(|path| path == source) {
            // a partial copy can leave the file behind before it fails
            fs::write(destination, b"partial").expect("the partial download");
            return Err(call_error("copy_from_runtime", "the copy was refused"));
        }
        let Some(content) = script.runtime_files.get(source) else {
            return Err(call_error("copy_from_runtime", "no such file"));
        };
        fs::write(destination, content).expect("the downloaded file");
        Ok(())
    }

    async fn run(&self, code: &str) -> Result<RunResult, SessionError> {
        {
            let mut calls = self.script.calls();
            calls.runs.push(code.to_owned());
        }
        let kind = kind_of(code);
        if kind == Kind::Execution {
            if let Some(hang) = self.script.hang {
                tokio::time::sleep(hang).await;
            }
        }
        let chosen = self.script.reply_for(kind);
        Ok(RunResult {
            exit_code: chosen.exit_code,
            stdout: chosen.stdout,
            stderr: chosen.stderr,
        })
    }

    async fn close(&self) -> Result<(), SessionError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let mut calls = self.script.calls();
        calls.closes += 1;
        match &self.script.close_error {
            Some(message) => Err(call_error("__exit__", message)),
            None => Ok(()),
        }
    }
}

impl Drop for ScriptedSession {
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            let mut calls = self.script.calls();
            calls.closes += 1;
        }
    }
}

struct ScriptedOpener {
    script: Arc<Script>,
}

#[async_trait]
impl SessionOpener for ScriptedOpener {
    async fn open(
        &self,
        options: &SessionOptions,
    ) -> Result<Box<dyn SandboxSession>, SessionError> {
        if self.script.missing_sdk {
            return Err(SessionError::SdkMissing);
        }
        if let Some(message) = &self.script.open_error {
            return Err(call_error("__enter__", message));
        }
        self.script.calls().options = Some(options.clone());
        Ok(Box::new(ScriptedSession {
            script: Arc::clone(&self.script),
            closed: AtomicBool::new(false),
        }))
    }
}

fn executor_for(script: &Arc<Script>) -> SandboxExecutor<ScriptedOpener> {
    SandboxExecutor {
        opener: ScriptedOpener {
            script: Arc::clone(script),
        },
        image: None,
        timeout: Duration::from_secs(30),
    }
}

fn skill_files(entries: &[(&str, &str)]) -> SkillFiles {
    entries
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.as_bytes().to_vec()))
        .collect()
}

async fn execute(
    executor: &SandboxExecutor<ScriptedOpener>,
    files: &SkillFiles,
    code: &str,
    requirements: Option<&str>,
) -> SandboxOutcome {
    executor
        .execute(SandboxRequest {
            code,
            skill_files: files,
            requirements,
        })
        .await
}

#[tokio::test]
async fn staged_files_reach_the_sandbox_under_sandbox_root() {
    let script = Arc::new(Script::default());
    let files = skill_files(&[("main.py", "print(1)\n"), ("pkg/tool.py", "x = 2\n")]);
    let outcome = execute(&executor_for(&script), &files, "main", None).await;
    assert!(outcome.success);
    assert_eq!(
        script.calls().copies,
        vec![
            ("/sandbox/main.py".to_owned(), b"print(1)\n".to_vec()),
            ("/sandbox/pkg/tool.py".to_owned(), b"x = 2\n".to_vec()),
        ]
    );
}

#[tokio::test]
async fn escaping_paths_are_skipped_before_they_are_copied() {
    let script = Arc::new(Script::default());
    let files = skill_files(&[
        ("main.py", "safe"),
        ("../escape.py", "up"),
        ("/etc/passwd", "absolute"),
        (".", "here"),
    ]);
    let outcome = execute(&executor_for(&script), &files, "main", None).await;
    assert!(outcome.success);
    let copies = script.calls().copies.clone();
    assert_eq!(
        copies
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        vec!["/sandbox/main.py"]
    );
}

#[tokio::test]
async fn the_request_requirements_win_over_the_skill_file() {
    let script = Arc::new(Script::default());
    let files = skill_files(&[("requirements.txt", "skill-package\n")]);
    let outcome = execute(
        &executor_for(&script),
        &files,
        "main",
        Some("request-package\n"),
    )
    .await;
    assert!(outcome.success);
    let calls = script.calls();
    assert!(calls.copies.contains(&(
        format!("{SANDBOX_ROOT}/{REQUIREMENTS_FILE}"),
        b"request-package\n".to_vec()
    )));
    assert_eq!(
        calls.runs.first(),
        Some(&pip_install_code(REQUIREMENTS_FILE))
    );
    assert_eq!(calls.runs.last(), Some(&wrapped_code("main")));
}

#[tokio::test]
async fn a_blank_request_requirements_falls_back_to_the_skill_file() {
    let script = Arc::new(Script::default());
    let files = skill_files(&[("requirements.txt", "skill-package\n")]);
    let outcome = execute(&executor_for(&script), &files, "main", Some("")).await;
    assert!(outcome.success);
    let calls = script.calls();
    assert!(
        !calls
            .copies
            .iter()
            .any(|(path, _)| path.ends_with(REQUIREMENTS_FILE))
    );
    assert_eq!(
        calls.runs.first(),
        Some(&pip_install_code(SKILL_REQUIREMENTS_FILE))
    );
}

#[tokio::test]
async fn the_skill_requirements_install_when_the_request_has_none() {
    let script = Arc::new(Script::default());
    let files = skill_files(&[("requirements.txt", "skill-package\n")]);
    let outcome = execute(&executor_for(&script), &files, "main", None).await;
    assert!(outcome.success);
    let calls = script.calls();
    assert_eq!(
        calls.runs.first(),
        Some(&pip_install_code(SKILL_REQUIREMENTS_FILE))
    );
    assert_eq!(
        calls.copies,
        vec![(
            "/sandbox/requirements.txt".to_owned(),
            b"skill-package\n".to_vec()
        )]
    );
}

#[tokio::test]
async fn no_requirements_runs_no_install() {
    let script = Arc::new(Script::default());
    let files = skill_files(&[("main.py", "print(1)")]);
    let outcome = execute(&executor_for(&script), &files, "main", None).await;
    assert!(outcome.success);
    assert_eq!(script.calls().runs, vec![wrapped_code("main")]);
}

#[tokio::test]
async fn a_failed_install_returns_its_output_and_skips_the_code() {
    let mut script = Script::default();
    script.install = Some(reply(1, Some("pip out"), Some("pip err")));
    let script = Arc::new(script);
    let files = skill_files(&[("requirements.txt", "pkg\n")]);
    let outcome = execute(&executor_for(&script), &files, "main", None).await;
    assert!(!outcome.success);
    assert_eq!(outcome.output, "pip out");
    assert_eq!(outcome.error, "pip err");
    assert!(outcome.files.is_empty());
    assert_eq!(
        script.calls().runs,
        vec![pip_install_code(SKILL_REQUIREMENTS_FILE)]
    );
}

#[tokio::test]
async fn a_successful_execution_returns_its_output() {
    let mut script = Script::default();
    script.execution = Some(reply(0, Some("hello"), Some("")));
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(outcome.success);
    assert_eq!(outcome.output, "hello");
    assert_eq!(outcome.error, "");
    assert!(outcome.files.is_empty());
}

#[tokio::test]
async fn a_failed_execution_keeps_its_generated_files() {
    let mut script = Script::default();
    script.execution = Some(reply(1, None, Some("boom")));
    script.listing = Some(reply(0, Some(r#"["/sandbox/chart.png"]"#), None));
    script
        .runtime_files
        .insert("/sandbox/chart.png".to_owned(), b"png-bytes".to_vec());
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(!outcome.success);
    assert_eq!(outcome.output, "");
    assert_eq!(outcome.error, "boom");
    assert_eq!(outcome.files.len(), 1);
    let collected = &outcome.files[0];
    assert_eq!(collected.name, "chart.png");
    assert_eq!(collected.path, "chart.png");
    assert_eq!(collected.content_base64, BASE64.encode(b"png-bytes"));
    assert_eq!(collected.mime_type, "image/png");
}

#[tokio::test]
async fn generated_files_exclude_the_original_skill_files() {
    let mut script = Script::default();
    script.listing = Some(reply(
        0,
        Some(r#"["/sandbox/main.py", "/sandbox/report.csv"]"#),
        None,
    ));
    script
        .runtime_files
        .insert("/sandbox/main.py".to_owned(), b"print(1)".to_vec());
    script
        .runtime_files
        .insert("/sandbox/report.csv".to_owned(), b"a,b\n".to_vec());
    let script = Arc::new(script);
    let files = skill_files(&[("main.py", "print(1)")]);
    let outcome = execute(&executor_for(&script), &files, "main", None).await;
    assert_eq!(outcome.files.len(), 1);
    assert_eq!(outcome.files[0].name, "report.csv");
    assert_eq!(outcome.files[0].path, "report.csv");
    assert_eq!(outcome.files[0].mime_type, "text/csv");
}

#[tokio::test]
async fn an_empty_listing_entry_is_skipped() {
    let mut script = Script::default();
    script.listing = Some(reply(0, Some(r#"["", "/sandbox/a.png"]"#), None));
    script
        .runtime_files
        .insert(String::new(), b"nothing".to_vec());
    script
        .runtime_files
        .insert("/sandbox/a.png".to_owned(), b"a".to_vec());
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert_eq!(outcome.files.len(), 1);
    assert_eq!(outcome.files[0].path, "a.png");
}

#[tokio::test]
async fn a_copy_failure_keeps_the_collected_prefix() {
    let mut script = Script::default();
    script.listing = Some(reply(
        0,
        Some(r#"["/sandbox/a.png", "/sandbox/broken.csv", "/sandbox/b.png"]"#),
        None,
    ));
    script
        .runtime_files
        .insert("/sandbox/a.png".to_owned(), b"/sandbox/a.png".to_vec());
    script
        .runtime_files
        .insert("/sandbox/b.png".to_owned(), b"/sandbox/b.png".to_vec());
    script
        .refusing_copies
        .push("/sandbox/broken.csv".to_owned());
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    let names: Vec<&str> = outcome
        .files
        .iter()
        .map(|collected| collected.name.as_str())
        .collect();
    assert_eq!(names, vec!["a.png", "b.png"]);
    assert_eq!(
        script.calls().downloaded,
        vec![
            "/sandbox/a.png".to_owned(),
            "/sandbox/broken.csv".to_owned(),
            "/sandbox/b.png".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_failing_listing_collects_nothing() {
    let mut script = Script::default();
    script.listing = Some(reply(2, Some(r#"["/sandbox/a.png"]"#), Some("walk")));
    script
        .runtime_files
        .insert("/sandbox/a.png".to_owned(), b"a".to_vec());
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(outcome.success);
    assert!(outcome.files.is_empty());
}

#[tokio::test]
async fn an_empty_listing_stdout_collects_nothing() {
    let mut script = Script::default();
    script.listing = Some(reply(0, Some(""), None));
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(outcome.files.is_empty());
}

#[tokio::test]
async fn a_malformed_listing_collects_nothing() {
    let mut script = Script::default();
    script.listing = Some(reply(0, Some("not json"), None));
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(outcome.files.is_empty());
}

#[tokio::test]
async fn a_missing_sdk_returns_the_install_message() {
    let mut script = Script::default();
    script.missing_sdk = true;
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(!outcome.success);
    assert_eq!(outcome.error, SDK_MISSING);
    assert_eq!(outcome.output, "");
    assert!(outcome.files.is_empty());
}

#[tokio::test]
async fn an_opening_error_returns_its_message() {
    let mut script = Script::default();
    script.open_error = Some("no docker daemon".to_owned());
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(!outcome.success);
    assert_eq!(
        outcome.error,
        "the sandbox call __enter__ failed: no docker daemon"
    );
    assert!(outcome.files.is_empty());
}

#[tokio::test]
async fn the_session_receives_python_and_the_configured_image() {
    let script = Arc::new(Script::default());
    let mut executor = executor_for(&script);
    executor.image = Some("ghcr.io/vndee/sandbox-python-311-bullseye".to_owned());
    execute(&executor, &skill_files(&[]), "main", None).await;
    assert_eq!(
        script.calls().options,
        Some(SessionOptions {
            lang: "python",
            verbose: false,
            image: Some("ghcr.io/vndee/sandbox-python-311-bullseye".to_owned()),
        })
    );
}

#[tokio::test]
async fn an_empty_image_is_not_passed() {
    let script = Arc::new(Script::default());
    let mut executor = executor_for(&script);
    executor.image = Some(String::new());
    execute(&executor, &skill_files(&[]), "main", None).await;
    assert_eq!(
        script.calls().options,
        Some(SessionOptions {
            lang: SESSION_LANG,
            verbose: false,
            image: None,
        })
    );
}

#[tokio::test]
async fn a_failing_close_replaces_the_result() {
    let mut script = Script::default();
    script
        .runtime_files
        .insert("/sandbox/a.png".to_owned(), b"a".to_vec());
    script.listing = Some(reply(0, Some(r#"["/sandbox/a.png"]"#), None));
    script.close_error = Some("docker stop failed".to_owned());
    let script = Arc::new(script);
    let outcome = execute(&executor_for(&script), &skill_files(&[]), "main", None).await;
    assert!(!outcome.success);
    assert_eq!(
        outcome.error,
        "the sandbox call __exit__ failed: docker stop failed"
    );
    assert!(outcome.files.is_empty());
    assert_eq!(script.calls().closes, 1);
}

#[tokio::test(start_paused = true)]
async fn a_timeout_cancellation_cleans_session_and_staging() {
    let mut script = Script::default();
    script.hang = Some(Duration::from_secs(600));
    let script = Arc::new(script);
    let mut executor = executor_for(&script);
    executor.timeout = Duration::from_secs(1);
    let files = skill_files(&[("main.py", "print(1)")]);
    let outcome = execute(&executor, &files, "main", None).await;
    assert!(!outcome.success);
    assert_eq!(
        outcome.error,
        "the sandbox execution did not finish within 1s"
    );
    assert!(outcome.files.is_empty());
    let calls = script.calls();
    assert_eq!(calls.closes, 1);
    assert_eq!(calls.staging.len(), 1);
    assert!(calls.staging.iter().all(|path| !path.exists()));
}
