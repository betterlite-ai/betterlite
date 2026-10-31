#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt as _;
use secrecy::SecretString;
use tempfile::TempDir;
use tokio::io::{AsyncWriteExt as _, duplex};
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::CLIHarnessHandler;
use super::io::{
    HARNESS_STREAM_CHUNK_BYTES, decode_lossy, drain_stderr, iter_stream_lines, push_tail, take_line,
};
use super::setup::{
    PERSIST_OUTPUT_LIMIT, PersistOutcome, apply_session_setup, copy_skills, persist_dir,
    persist_outcome, sandbox_path,
};
use super::turn_io::{TurnCursor, TurnProgress};
use crate::context::{SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::handlers::BaseHarnessHandler as _;
use crate::options::OpenCodeOptions;
use crate::payloads::decode_json_line;
use crate::sandbox::local::LocalSandbox;
use crate::sandbox::{Process as _, Sandbox, SandboxCommand};
use crate::transformation::{
    BaseCLIHarnessConfig, BaseHarnessConfig, HarnessSessionSetup, HarnessTurnRequest,
    HarnessTurnResponse, PersistedDir, TurnInput,
};
use crate::types::{Capabilities, Harness, JsonObject};

// round-trips through serde so the test double never names the disallowed serde_json::Value
fn json_string(line: &JsonObject, key: &str) -> Option<String> {
    let raw = line.get(key)?.to_string();
    serde_json::from_str(&raw).ok()
}

#[derive(Clone, Default)]
struct TestState {
    native_id: Option<String>,
    texts: Vec<String>,
}

#[derive(Clone, Default)]
struct TestConfig {
    binary: &'static str,
    argv: Vec<String>,
    files: BTreeMap<String, Vec<u8>>,
    persisted_dirs: Vec<PersistedDir>,
    skills_dir: Option<String>,
}

impl BaseHarnessConfig for TestConfig {
    type Options = OpenCodeOptions;
    const HARNESS: Harness = Harness::OpenCode;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::new(),
            permission_modes: BTreeSet::new(),
        }
    }
}

impl BaseCLIHarnessConfig for TestConfig {
    type StreamState = TestState;

    fn get_binary(&self) -> &'static str {
        if self.binary.is_empty() {
            "sh"
        } else {
            self.binary
        }
    }

    fn get_install_hint(&self) -> &'static str {
        "install the test binary"
    }

    fn transform_session_setup<S>(
        &self,
        _context: &SessionContext<S>,
        _private_dir: &str,
    ) -> Result<HarnessSessionSetup, HarnessError>
    where
        S: Sandbox,
    {
        Ok(HarnessSessionSetup {
            files: self.files.clone(),
            persisted_dirs: self.persisted_dirs.clone(),
            skills_dir: self.skills_dir.clone(),
            env: BTreeMap::new(),
        })
    }

    fn transform_turn_request<S>(
        &self,
        _context: &SessionContext<S>,
        turn: TurnInput<'_>,
    ) -> Result<HarnessTurnRequest, HarnessError>
    where
        S: Sandbox,
    {
        let stdin = turn.prompt.to_owned();
        let request = HarnessTurnRequest {
            argv: self.argv.clone(),
            env: BTreeMap::new(),
            stdin,
            cwd: None,
        };
        Ok(request)
    }

    fn create_stream_state(&self) -> TestState {
        TestState::default()
    }

    fn transform_stream_line(&self, line: &JsonObject, state: &mut TestState) -> Vec<Event> {
        if let Some(id) = json_string(line, "session_id") {
            state.native_id = Some(id);
        }
        json_string(line, "text")
            .map(|text| {
                state.texts.push(text.clone());
                vec![Event::Text { delta: text }]
            })
            .unwrap_or_default()
    }

    fn get_native_session_id(&self, state: &TestState) -> Option<String> {
        state.native_id.clone()
    }

    fn transform_turn_response<S>(
        &self,
        _context: &SessionContext<S>,
        state: &TestState,
        exit_code: i32,
        stderr_tail: &[String],
    ) -> Result<HarnessTurnResponse, HarnessError> {
        Ok(HarnessTurnResponse {
            final_text: state.texts.join(""),
            output_json: Some(format!("exit={exit_code};tail={}", stderr_tail.join("|"))),
        })
    }
}

fn tempdir() -> TempDir {
    tempfile::Builder::new()
        .prefix("betterlite-cli-handler-test-")
        .tempdir()
        .unwrap()
}

fn sandbox_with_env(root: &Path, env: &BTreeMap<String, SecretString>) -> LocalSandbox {
    LocalSandbox::new(root, env, root.to_path_buf()).unwrap()
}

fn context(sandbox: LocalSandbox) -> SessionContext<LocalSandbox> {
    SessionContext {
        harness: Harness::OpenCode,
        sandbox,
        session_id: "session".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    }
}

#[tokio::test]
async fn start_fails_when_the_binary_is_missing() {
    let root = tempdir();
    let mut ctx = context(sandbox_with_env(root.path(), &BTreeMap::new()));
    let mut handler = CLIHarnessHandler {
        config: TestConfig {
            binary: "betterlite-test-binary-that-does-not-exist",
            argv: Vec::new(),
            files: BTreeMap::new(),
            persisted_dirs: Vec::new(),
            skills_dir: None,
        },
        private_dir: None,
        setup: None,
        native_id: None,
    };
    let error = handler.start(&mut ctx).await.unwrap_err();
    assert!(matches!(error, HarnessError::InstallFailed(_)));
}

#[tokio::test]
async fn start_writes_files_and_links_the_persisted_cache_dir() {
    let root = tempdir();
    let home = tempdir();
    let mut env = BTreeMap::new();
    env.insert(
        "HOME".to_owned(),
        SecretString::from(home.path().to_str().unwrap()),
    );
    let mut ctx = context(sandbox_with_env(root.path(), &env));
    let mut files = BTreeMap::new();
    files.insert("config.json".to_owned(), b"{}".to_vec());
    let mut handler = CLIHarnessHandler {
        config: TestConfig {
            binary: "sh",
            files,
            persisted_dirs: vec![PersistedDir {
                private_path: "cache-link".to_owned(),
                cache_path: "cli-handler-test".to_owned(),
            }],
            argv: Vec::new(),
            skills_dir: None,
        },
        private_dir: None,
        setup: None,
        native_id: None,
    };
    handler.start(&mut ctx).await.unwrap();
    let private_dir = handler.private_dir.clone().unwrap();
    let data = ctx
        .sandbox
        .read(&format!("{private_dir}/config.json"), 16)
        .await
        .unwrap();
    assert_eq!(data, b"{}");
    let link = fs::read_link(format!("{private_dir}/cache-link")).unwrap();
    assert!(link.ends_with("cli-handler-test"));
}

#[tokio::test]
async fn turn_streams_a_long_stdout_line_without_truncation_and_tracks_the_session_id() {
    let root = tempdir();
    let mut ctx = context(sandbox_with_env(root.path(), &BTreeMap::new()));
    let long_text = "x".repeat(70_000);
    let body = format!("{{\"session_id\":\"abc\"}}\n{{\"text\":\"{long_text}\"}}\n");
    ctx.sandbox
        .write("turn.jsonl", body.as_bytes())
        .await
        .unwrap();
    let mut handler = CLIHarnessHandler {
        config: TestConfig {
            binary: "sh",
            argv: vec!["cat".to_owned(), "turn.jsonl".to_owned()],
            files: BTreeMap::new(),
            persisted_dirs: Vec::new(),
            skills_dir: None,
        },
        private_dir: None,
        setup: None,
        native_id: None,
    };
    handler.start(&mut ctx).await.unwrap();
    let events = timeout(
        Duration::from_secs(5),
        handler
            .turn(&mut ctx, "prompt")
            .map(Result::unwrap)
            .collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    assert_eq!(events.len(), 1);
    let Event::Text { delta } = events.first().unwrap() else {
        panic!("expected a text event");
    };
    assert_eq!(delta.len(), 70_000);
    assert_eq!(handler.native_session_id(), Some("abc".to_owned()));
    assert_eq!(ctx.outcome.output_json, Some("exit=0;tail=".to_owned()));
}

#[tokio::test]
async fn turn_drains_stderr_concurrently_so_a_full_pipe_cannot_deadlock_stdout() {
    let root = tempdir();
    let mut ctx = context(sandbox_with_env(root.path(), &BTreeMap::new()));
    // fills the stderr pipe well before stdout closes, so sequential draining would deadlock
    let script = "i=0; while [ $i -lt 20000 ]; do echo stderr-line-$i >&2; i=$((i+1)); done; echo '{\"text\":\"done\"}'";
    let mut handler = CLIHarnessHandler {
        config: TestConfig {
            binary: "sh",
            argv: vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()],
            files: BTreeMap::new(),
            persisted_dirs: Vec::new(),
            skills_dir: None,
        },
        private_dir: None,
        setup: None,
        native_id: None,
    };
    handler.start(&mut ctx).await.unwrap();
    let events = timeout(
        Duration::from_secs(10),
        handler
            .turn(&mut ctx, "prompt")
            .map(Result::unwrap)
            .collect::<Vec<_>>(),
    )
    .await
    .expect("stdout/stderr draining must not deadlock");
    assert_eq!(events.len(), 1);
    let tail = ctx.outcome.output_json.clone().unwrap();
    assert!(tail.contains("stderr-line-19999"));
    assert!(!tail.contains("stderr-line-0,"));
}

#[tokio::test]
async fn turn_progress_finishes_after_both_pipes_close_without_data() {
    let root = tempdir();
    let sandbox = sandbox_with_env(root.path(), &BTreeMap::new());
    let command = SandboxCommand {
        argv: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "exec 1>&-; exec 2>&-".to_owned(),
        ],
        env: BTreeMap::new(),
        cwd: None,
    };
    let mut process = sandbox.exec(&command).await.unwrap();
    let mut cursor = TurnCursor::default();
    let progress = timeout(
        Duration::from_secs(5),
        super::turn_io::next_turn_progress(&mut process, &mut cursor),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(progress.is_none());
    process.wait().await.unwrap();
}

#[tokio::test]
async fn turn_progress_reads_stderr_after_stdout_closes() {
    let root = tempdir();
    let sandbox = sandbox_with_env(root.path(), &BTreeMap::new());
    let command = SandboxCommand {
        argv: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "exec 1>&-; sleep 0.1; printf late >&2".to_owned(),
        ],
        env: BTreeMap::new(),
        cwd: None,
    };
    let mut process = sandbox.exec(&command).await.unwrap();
    let mut cursor = TurnCursor::default();
    let stderr = timeout(Duration::from_secs(5), async {
        let progress = super::turn_io::next_turn_progress(&mut process, &mut cursor)
            .await
            .unwrap();
        match progress {
            Some(TurnProgress::Stderr(data)) => data,
            Some(TurnProgress::Stdout(_)) => panic!("stdout should already be closed"),
            None => panic!("stderr must be read before both pipes close"),
        }
    })
    .await
    .unwrap();
    assert_eq!(stderr, b"late");
    process.wait().await.unwrap();
}

#[tokio::test]
async fn turn_progress_does_not_emit_an_empty_stderr_line_after_eof() {
    let root = tempdir();
    let sandbox = sandbox_with_env(root.path(), &BTreeMap::new());
    let command = SandboxCommand {
        argv: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "exec 1>&-; exec 2>&-".to_owned(),
        ],
        env: BTreeMap::new(),
        cwd: None,
    };
    let mut process = sandbox.exec(&command).await.unwrap();
    let mut cursor = TurnCursor::default();
    let progress = timeout(
        Duration::from_secs(5),
        super::turn_io::next_turn_progress(&mut process, &mut cursor),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(progress.is_none());
    process.wait().await.unwrap();
}

// Fails today: AsyncWriteExt::shutdown() is a no-op on tokio's ChildStdin, see the task report.
#[tokio::test]
async fn turn_closes_stdin_before_reading_stdout() {
    let root = tempdir();
    let mut ctx = context(sandbox_with_env(root.path(), &BTreeMap::new()));
    // cat echoes stdin back only after it sees EOF on it
    let mut handler = CLIHarnessHandler {
        config: TestConfig {
            binary: "cat",
            argv: vec!["cat".to_owned()],
            files: BTreeMap::new(),
            persisted_dirs: Vec::new(),
            skills_dir: None,
        },
        private_dir: None,
        setup: None,
        native_id: None,
    };
    handler.start(&mut ctx).await.unwrap();
    let events = timeout(
        Duration::from_secs(5),
        handler
            .turn(&mut ctx, "{\"text\":\"echoed\"}\n")
            .map(Result::unwrap)
            .collect::<Vec<_>>(),
    )
    .await
    .expect("a process that waits for stdin EOF must not hang");
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn stop_is_safe_to_call_twice() {
    let root = tempdir();
    let mut ctx = context(sandbox_with_env(root.path(), &BTreeMap::new()));
    let mut handler = CLIHarnessHandler {
        config: TestConfig {
            binary: "sh",
            argv: Vec::new(),
            files: BTreeMap::new(),
            persisted_dirs: Vec::new(),
            skills_dir: None,
        },
        private_dir: None,
        setup: None,
        native_id: None,
    };
    handler.start(&mut ctx).await.unwrap();
    handler.stop(&mut ctx).await.unwrap();
    handler.stop(&mut ctx).await.unwrap();
}

#[tokio::test]
async fn resume_sets_the_native_session_id_used_by_the_next_turn_request() {
    let root = tempdir();
    let mut ctx = context(sandbox_with_env(root.path(), &BTreeMap::new()));
    let mut handler = CLIHarnessHandler {
        config: TestConfig {
            binary: "sh",
            argv: Vec::new(),
            files: BTreeMap::new(),
            persisted_dirs: Vec::new(),
            skills_dir: None,
        },
        private_dir: None,
        setup: None,
        native_id: None,
    };
    assert_eq!(handler.native_session_id(), None);
    handler.resume(&mut ctx, "resumed-id").await.unwrap();
    assert_eq!(handler.native_session_id(), Some("resumed-id".to_owned()));
}

#[tokio::test]
async fn iter_stream_lines_splits_chunks_arriving_separately_and_in_one_read() {
    let (mut writer, reader) = duplex(64);
    let mut writing = JoinSet::new();
    writing.spawn(async move {
        writer.write_all(b"line-a\nlin").await.unwrap();
        writer.write_all(b"e-b\nline-c").await.unwrap();
        writer.shutdown().await.unwrap();
    });
    let lines = timeout(Duration::from_secs(5), iter_stream_lines(reader))
        .await
        .unwrap()
        .unwrap();
    writing.join_next().await.unwrap().unwrap();
    assert_eq!(
        lines,
        vec![b"line-a".to_vec(), b"line-b".to_vec(), b"line-c".to_vec()]
    );
}

#[tokio::test]
async fn iter_stream_lines_keeps_the_final_partial_line_without_a_trailing_newline() {
    let (mut writer, reader) = duplex(64);
    writer.write_all(b"only-line-no-newline").await.unwrap();
    writer.shutdown().await.unwrap();
    let lines = timeout(Duration::from_secs(5), iter_stream_lines(reader))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lines, vec![b"only-line-no-newline".to_vec()]);
}

#[tokio::test]
async fn drain_stderr_decodes_invalid_utf8_with_replacement() {
    let (mut writer, reader) = duplex(64);
    writer.write_all(b"bad-\xff-byte\n").await.unwrap();
    writer.shutdown().await.unwrap();
    let mut tail = VecDeque::new();
    timeout(Duration::from_secs(5), drain_stderr(reader, &mut tail))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tail.as_slices().0, ["bad-\u{fffd}-byte".to_owned()]);
    assert_eq!(decode_lossy(b"bad-\xff-byte"), "bad-\u{fffd}-byte");
}

#[test]
fn stream_chunk_size_is_64_kibibytes() {
    assert_eq!(HARNESS_STREAM_CHUNK_BYTES, 64 * 1024);
    assert_eq!(PERSIST_OUTPUT_LIMIT, 16 * 1024);
}

#[test]
fn push_tail_drops_the_oldest_line_once_the_ring_is_full() {
    let mut tail = VecDeque::new();
    (0_usize..45_usize).for_each(|index| push_tail(&mut tail, format!("line-{index}")));
    assert_eq!(tail.len(), 40);
    assert_eq!(tail.front(), Some(&"line-5".to_owned()));
    assert_eq!(tail.back(), Some(&"line-44".to_owned()));
}

#[test]
fn persist_outcome_recognizes_success_and_failure_exit_codes() {
    assert_eq!(persist_outcome(0_i32), PersistOutcome::Complete);
    assert_eq!(persist_outcome(1_i32), PersistOutcome::Failed);
}

#[test]
fn sandbox_path_prepends_the_private_dir_only_for_relative_paths() {
    assert_eq!(
        sandbox_path("/private", "config.json"),
        "/private/config.json"
    );
    assert_eq!(
        sandbox_path("/private", "/abs/config.json"),
        "/abs/config.json"
    );
}

#[tokio::test]
async fn persist_dir_logs_and_continues_when_the_script_fails() {
    let root = tempdir();
    // HOME points at a file, not a directory, so `mkdir -p "$HOME/.cache/..."` fails
    let not_a_dir = root.path().join("not-a-dir");
    fs::write(&not_a_dir, b"").unwrap();
    let mut env = BTreeMap::new();
    env.insert(
        "HOME".to_owned(),
        SecretString::from(not_a_dir.to_str().unwrap()),
    );
    let sandbox = sandbox_with_env(root.path(), &env);
    persist_dir(&sandbox, "cache-link", "whatever")
        .await
        .unwrap();
}

#[tokio::test]
async fn copy_skills_resolves_the_skill_name_and_converts_separators() {
    let root = tempdir();
    let skill = tempdir();
    fs::write(skill.path().join("SKILL.md"), b"# skill").unwrap();
    fs::create_dir_all(skill.path().join("nested")).unwrap();
    fs::write(skill.path().join("nested").join("file.txt"), b"hi").unwrap();
    let sandbox = sandbox_with_env(root.path(), &BTreeMap::new());
    copy_skills(
        &sandbox,
        &[skill.path().to_str().unwrap().to_owned()],
        "skills",
    )
    .await
    .unwrap();
    let name = skill.path().file_name().unwrap().to_str().unwrap();
    let data = sandbox
        .read(&format!("skills/{name}/nested/file.txt"), 16)
        .await
        .unwrap();
    assert_eq!(data, b"hi");
}

#[tokio::test]
async fn apply_session_setup_skips_empty_skill_lists() {
    let root = tempdir();
    let sandbox = sandbox_with_env(root.path(), &BTreeMap::new());
    let setup = HarnessSessionSetup {
        files: BTreeMap::new(),
        persisted_dirs: Vec::new(),
        skills_dir: Some("skills".to_owned()),
        env: BTreeMap::new(),
    };
    apply_session_setup(&sandbox, "private", &setup, &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn apply_session_setup_reads_nonempty_skill_lists() {
    let root = tempdir();
    let sandbox = sandbox_with_env(root.path(), &BTreeMap::new());
    let setup = HarnessSessionSetup {
        files: BTreeMap::new(),
        persisted_dirs: Vec::new(),
        skills_dir: Some("skills".to_owned()),
        env: BTreeMap::new(),
    };
    let result = apply_session_setup(
        &sandbox,
        "private",
        &setup,
        &["missing-skill-for-cli-handler-test".to_owned()],
    )
    .await;
    assert!(result.is_err());
}

#[test]
fn take_line_removes_only_one_complete_line() {
    let mut buffer = b"first\nsecond".to_vec();
    assert_eq!(take_line(&mut buffer), Some(b"first".to_vec()));
    assert_eq!(buffer, b"second");
}

// Fails today for the same reason as turn_closes_stdin_before_reading_stdout.
#[tokio::test]
async fn send_stdin_through_a_real_process_writes_and_closes() {
    let root = tempdir();
    let sandbox = sandbox_with_env(root.path(), &BTreeMap::new());
    let command = SandboxCommand {
        argv: vec!["cat".to_owned()],
        env: BTreeMap::new(),
        cwd: None,
    };
    let mut proc = sandbox.exec(&command).await.unwrap();
    super::io::send_stdin(&mut proc, "hello").await.unwrap();
    let lines = timeout(Duration::from_secs(5), iter_stream_lines(proc.stdout()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lines, vec![b"hello".to_vec()]);
    proc.wait().await.unwrap();
}

async fn replay_corpus_entries(root: &Path) -> usize {
    let entries = fs::read_dir(root).unwrap();
    let mut count = 0_usize;
    for entry in entries {
        let path = entry.unwrap().path();
        let bytes = fs::read(path).unwrap();
        let lines = timeout(Duration::from_secs(5), iter_stream_lines(&*bytes))
            .await
            .expect("iter_stream_lines must not hang")
            .unwrap();
        for line in lines {
            drop(decode_json_line(&line));
        }
        count = count.saturating_add(1);
    }
    count
}

#[tokio::test]
async fn replays_the_cli_jsonl_framing_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cli_jsonl_framing");
    assert!(replay_corpus_entries(&root).await > 0);
}
