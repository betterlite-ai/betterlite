#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use betterlite_config::loader::EnvironmentLookup;
use chrono::{DateTime, Utc};
use clap::Parser;
use rstest::rstest;

use super::error::DebugError;
use super::render::Clock;
use super::wire::{RequestResponsePayload, SessionLogsPage};
use super::{ClaudeOptions, DebugCommand, DebugPorts, run};
use crate::cli_context::CliContext;
use crate::commands::tests::support::{Answer, RecordedRequest, TestServer, client};

const SESSION_UUID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const UNUSED_PROXY: &str = "http://127.0.0.1:1";

#[derive(Debug, Parser)]
struct Wrapper {
    #[command(subcommand)]
    command: DebugCommand,
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_767_225_600, 0).unwrap()
    }
}

struct Home {
    home: Option<PathBuf>,
    variables: BTreeMap<String, String>,
}

impl EnvironmentLookup for Home {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

struct BrokenPipe;

impl Write for BrokenPipe {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Outcome {
    result: Result<(), DebugError>,
    stdout: String,
    stderr: String,
}

fn home_with(directory: &Path, variables: &[(&str, &str)]) -> Home {
    Home {
        home: Some(directory.to_owned()),
        variables: variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn claude_options(args: &[&str]) -> ClaudeOptions {
    let mut argv = vec!["lite", "claude"];
    argv.extend_from_slice(args);
    match Wrapper::try_parse_from(argv).unwrap().command {
        DebugCommand::Claude(options) => options,
        DebugCommand::InstallClaudeCommand => panic!("parsed the wrong subcommand"),
    }
}

fn run_with_writers(
    args: &[&str],
    environment: &Home,
    base_url: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), DebugError> {
    let mut argv = vec!["lite"];
    argv.extend_from_slice(args);
    let command = Wrapper::try_parse_from(argv).unwrap().command;
    let context = CliContext {
        base_url: base_url.to_owned(),
        ..CliContext::default()
    };
    let client = client(base_url, None);
    run(
        command,
        DebugPorts {
            context: &context,
            client: &client,
            environment,
            clock: &FixedClock,
            stdout,
            stderr,
        },
    )
}

fn run_command(args: &[&str], environment: &Home, base_url: &str) -> Outcome {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_writers(args, environment, base_url, &mut stdout, &mut stderr);
    Outcome {
        result,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn page(rows: &str) -> Answer {
    Answer::json(
        200,
        &format!(r#"{{"data":[{rows}],"total":0,"total_pages":1}}"#),
    )
}

#[test]
fn debug_claude_defaults_to_three_recent_bodies_and_twenty_thousand_characters() {
    let options = claude_options(&[]);

    assert_eq!(
        (
            options.session_id,
            options.recent_bodies,
            options.max_body_chars,
            options.no_save
        ),
        (None, 3, 20_000, false)
    );
}

#[test]
fn debug_claude_reads_every_option() {
    let options = claude_options(&[
        "--session-id",
        "sess-1",
        "--recent-bodies",
        "0",
        "--max-body-chars",
        "100",
        "--no-save",
    ]);

    assert_eq!(
        (
            options.session_id.as_deref(),
            options.recent_bodies,
            options.max_body_chars,
            options.no_save
        ),
        (Some("sess-1"), 0, 100, true)
    );
}

#[rstest]
#[case::zero_bodies(&["--recent-bodies=0"])]
#[case::many_bodies(&["--recent-bodies=4294967295"])]
#[case::lowest_body_limit(&["--max-body-chars=100"])]
#[case::above_the_lowest_body_limit(&["--max-body-chars=101"])]
fn debug_claude_accepts_values_inside_the_limits(#[case] args: &[&str]) {
    let mut argv = vec!["lite", "claude"];
    argv.extend_from_slice(args);

    assert!(Wrapper::try_parse_from(argv).is_ok());
}

#[rstest]
#[case::negative_bodies(&["--recent-bodies=-1"])]
#[case::word_for_bodies(&["--recent-bodies=many"])]
#[case::too_many_bodies(&["--recent-bodies=4294967296"])]
#[case::body_limit_below_the_lowest(&["--max-body-chars=99"])]
#[case::zero_body_limit(&["--max-body-chars=0"])]
#[case::negative_body_limit(&["--max-body-chars=-5"])]
#[case::word_for_body_limit(&["--max-body-chars=all"])]
#[case::unknown_option(&["--verbose"])]
fn debug_claude_rejects_values_outside_the_limits(#[case] args: &[&str]) {
    let mut argv = vec!["lite", "claude"];
    argv.extend_from_slice(args);

    assert!(Wrapper::try_parse_from(argv).is_err());
}

#[test]
fn debug_install_claude_command_parses_without_arguments() {
    let parsed = Wrapper::try_parse_from(["lite", "install-claude-command"]).unwrap();

    assert!(matches!(parsed.command, DebugCommand::InstallClaudeCommand));
}

#[test]
fn debug_claude_prints_the_report_of_the_given_session_and_saves_nothing_with_no_save() {
    let home = tempfile::tempdir().unwrap();
    let server = TestServer::start(vec![page(r#"{"request_id":"r1"}"#)]);
    let base_url = server.base_url.clone();

    let outcome = run_command(
        &[
            "claude",
            "--session-id",
            "sess-1",
            "--recent-bodies",
            "0",
            "--no-save",
        ],
        &home_with(home.path(), &[]),
        &base_url,
    );

    let requests = server.finish();
    outcome.result.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].path(),
        "/spend/logs/session/ui?session_id=sess-1&page=1&page_size=100"
    );
    assert!(
        outcome
            .stdout
            .starts_with("# LiteLLM debug report: Claude Code session `sess-1`\n")
    );
    assert!(outcome.stdout.contains(&format!("\n- proxy: {base_url}\n")));
    assert!(
        outcome
            .stdout
            .contains("\n- generated: 2026-01-01T00:00:00+00:00\n")
    );
    assert!(outcome.stdout.ends_with("tokens: 0 in / 0 out\n\n"));
    assert_eq!(outcome.stderr, "");
    assert!(!home.path().join(".litellm").exists());
}

#[test]
fn debug_claude_saves_a_private_copy_and_says_where() {
    let home = tempfile::tempdir().unwrap();
    let server = TestServer::start(vec![page(r#"{"request_id":"r1"}"#)]);

    let outcome = run_command(
        &["claude", "--session-id", "sess-1", "--recent-bodies", "0"],
        &home_with(home.path(), &[]),
        &server.base_url,
    );

    server.finish();
    outcome.result.unwrap();
    let saved = home
        .path()
        .join(".litellm")
        .join("debug")
        .join("claude-sess-1.md");
    assert_eq!(
        format!("{}\n", fs::read_to_string(&saved).unwrap()),
        outcome.stdout
    );
    assert_eq!(outcome.stderr, format!("Saved to {}\n", saved.display()));
    assert_eq!(
        fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[rstest]
#[case::environment_variable(&[("CLAUDE_CODE_SESSION_ID", "from-env")], None, "from-env")]
#[case::explicit_beats_environment(&[("CLAUDE_CODE_SESSION_ID", "from-env")], Some("explicit"), "explicit")]
#[case::empty_explicit_falls_back(&[("CLAUDE_CODE_SESSION_ID", "from-env")], Some(""), "from-env")]
#[case::transcript(&[], None, SESSION_UUID)]
fn debug_claude_picks_the_session_to_report(
    #[case] variables: &[(&str, &str)],
    #[case] explicit: Option<&str>,
    #[case] expected: &str,
) {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join(".claude").join("projects").join("p");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join(format!("{SESSION_UUID}.jsonl")), "").unwrap();
    let server = TestServer::start(vec![page(r#"{"request_id":"r1"}"#)]);
    let mut args = vec!["claude", "--recent-bodies", "0", "--no-save"];
    if let Some(id) = explicit {
        args.extend(["--session-id", id]);
    }

    let outcome = run_command(&args, &home_with(home.path(), variables), &server.base_url);

    let requests = server.finish();
    outcome.result.unwrap();
    assert_eq!(
        requests[0].path(),
        format!("/spend/logs/session/ui?session_id={expected}&page=1&page_size=100")
    );
}

#[rstest]
#[case::nothing_given(&[])]
#[case::empty_explicit(&["--session-id", ""])]
fn debug_claude_stops_when_no_session_can_be_found(#[case] extra: &[&str]) {
    let home = tempfile::tempdir().unwrap();
    let mut args = vec!["claude"];
    args.extend_from_slice(extra);

    let outcome = run_command(&args, &home_with(home.path(), &[]), UNUSED_PROXY);

    assert!(matches!(outcome.result, Err(DebugError::NoSession)));
    assert_eq!((outcome.stdout.as_str(), outcome.stderr.as_str()), ("", ""));
    assert!(!home.path().join(".litellm").exists());
}

#[rstest]
#[case::claude(&["claude", "--session-id", "s"])]
#[case::install(&["install-claude-command"])]
fn debug_claude_needs_a_home_directory(#[case] args: &[&str]) {
    let environment = Home {
        home: None,
        variables: BTreeMap::new(),
    };

    let outcome = run_command(args, &environment, UNUSED_PROXY);

    assert!(matches!(outcome.result, Err(DebugError::NoHome)));
    assert_eq!((outcome.stdout.as_str(), outcome.stderr.as_str()), ("", ""));
}

#[test]
fn debug_claude_stops_when_the_session_has_no_spend_logs() {
    let home = tempfile::tempdir().unwrap();
    let server = TestServer::start(vec![page("")]);
    let base_url = server.base_url.clone();

    let outcome = run_command(
        &["claude", "--session-id", "empty"],
        &home_with(home.path(), &[]),
        &base_url,
    );

    server.finish();
    let Err(DebugError::NoSessionRows {
        session_id,
        base_url: reported_url,
    }) = outcome.result
    else {
        panic!("expected NoSessionRows");
    };
    assert_eq!(
        (session_id.as_str(), reported_url.as_str()),
        ("empty", base_url.as_str())
    );
    assert_eq!((outcome.stdout.as_str(), outcome.stderr.as_str()), ("", ""));
    assert!(!home.path().join(".litellm").exists());
}

#[test]
fn debug_claude_stops_when_the_proxy_answers_with_an_error() {
    let home = tempfile::tempdir().unwrap();
    let server = TestServer::start(vec![Answer::text(401, "bad key")]);

    let outcome = run_command(
        &["claude", "--session-id", "s"],
        &home_with(home.path(), &[]),
        &server.base_url,
    );

    server.finish();
    assert!(matches!(
        outcome.result,
        Err(DebugError::Status { status: 401, .. })
    ));
    assert_eq!(outcome.stdout, "");
    assert!(!home.path().join(".litellm").exists());
}

#[rstest]
#[case::truncated_bodies(&["--max-body-chars", "100"], 3)]
#[case::whole_bodies(&[], 0)]
fn debug_claude_loads_the_bodies_of_the_recent_turns_and_cuts_them_to_the_limit(
    #[case] extra: &[&str],
    #[case] truncated: usize,
) {
    let home = tempfile::tempdir().unwrap();
    let long_body = format!(r#"{{"response":"{}"}}"#, "x".repeat(150));
    let server = TestServer::start(vec![
        page(r#"{"request_id":"r1"},{"request_id":"r2"},{"request_id":"r3"},{"request_id":"r4"}"#),
        Answer::json(200, &long_body),
        Answer::json(200, &long_body),
        Answer::json(200, &long_body),
    ]);
    let mut args = vec!["claude", "--session-id", "s", "--no-save"];
    args.extend_from_slice(extra);

    let outcome = run_command(&args, &home_with(home.path(), &[]), &server.base_url);

    let requests = server.finish();
    outcome.result.unwrap();
    let paths: Vec<&str> = requests.iter().map(RecordedRequest::path).skip(1).collect();
    assert_eq!(
        paths,
        [
            "/spend/logs/ui/r2",
            "/spend/logs/ui/r3",
            "/spend/logs/ui/r4"
        ]
    );
    assert_eq!(
        outcome
            .stdout
            .matches("... (truncated, 50 more chars)")
            .count(),
        truncated
    );
    assert_eq!(
        outcome
            .stdout
            .matches("<summary>response</summary>")
            .count(),
        3
    );
}

#[test]
fn debug_claude_loads_the_body_of_a_failed_turn_with_no_recent_bodies() {
    let home = tempfile::tempdir().unwrap();
    let server = TestServer::start(vec![
        page(r#"{"request_id":"bad","status":"failure"},{"request_id":"good"}"#),
        Answer::json(200, r#"{"response":"boom"}"#),
    ]);

    let outcome = run_command(
        &[
            "claude",
            "--session-id",
            "s",
            "--recent-bodies",
            "0",
            "--no-save",
        ],
        &home_with(home.path(), &[]),
        &server.base_url,
    );

    let requests = server.finish();
    outcome.result.unwrap();
    assert_eq!(requests[1].path(), "/spend/logs/ui/bad");
    assert_eq!(requests.len(), 2);
    assert!(outcome.stdout.contains("\n```json\nboom\n```\n"));
}

#[test]
fn debug_claude_reports_a_terminal_that_cannot_be_written_and_saves_nothing() {
    let home = tempfile::tempdir().unwrap();
    let server = TestServer::start(vec![page(r#"{"request_id":"r1"}"#)]);

    let result = run_with_writers(
        &["claude", "--session-id", "s", "--recent-bodies", "0"],
        &home_with(home.path(), &[]),
        &server.base_url,
        &mut BrokenPipe,
        &mut Vec::<u8>::new(),
    );

    server.finish();
    assert!(matches!(result, Err(DebugError::Output(_))));
    assert!(!home.path().join(".litellm").exists());
}

#[test]
fn debug_claude_reports_a_stderr_that_cannot_be_written_after_saving() {
    let home = tempfile::tempdir().unwrap();
    let server = TestServer::start(vec![page(r#"{"request_id":"r1"}"#)]);

    let result = run_with_writers(
        &["claude", "--session-id", "s", "--recent-bodies", "0"],
        &home_with(home.path(), &[]),
        &server.base_url,
        &mut Vec::<u8>::new(),
        &mut BrokenPipe,
    );

    server.finish();
    assert!(matches!(result, Err(DebugError::Output(_))));
    assert!(home.path().join(".litellm/debug/claude-s.md").is_file());
}

#[test]
fn debug_claude_reports_a_report_directory_it_cannot_create() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join(".litellm"), "").unwrap();
    let server = TestServer::start(vec![page(r#"{"request_id":"r1"}"#)]);

    let outcome = run_command(
        &["claude", "--session-id", "s", "--recent-bodies", "0"],
        &home_with(home.path(), &[]),
        &server.base_url,
    );

    server.finish();
    assert!(matches!(
        outcome.result,
        Err(DebugError::ReportDirectory { .. })
    ));
}

#[test]
fn debug_install_claude_command_installs_the_command_and_tells_where() {
    let home = tempfile::tempdir().unwrap();

    let outcome = run_command(
        &["install-claude-command"],
        &home_with(home.path(), &[]),
        UNUSED_PROXY,
    );

    outcome.result.unwrap();
    let installed = home.path().join(".claude/commands/debug-lite.md");
    assert!(
        fs::read_to_string(&installed)
            .unwrap()
            .contains("!`lite debug claude $ARGUMENTS`")
    );
    assert_eq!(
        outcome.stdout,
        format!(
            "Installed /debug-lite: {}\nRestart Claude Code (or start a new session), then type /debug-lite.\n",
            installed.display()
        )
    );
    assert_eq!(outcome.stderr, "");
}

#[test]
fn debug_install_claude_command_reports_a_commands_path_that_is_a_file() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(home.path().join(".claude/commands"), "").unwrap();

    let outcome = run_command(
        &["install-claude-command"],
        &home_with(home.path(), &[]),
        UNUSED_PROXY,
    );

    assert!(matches!(
        outcome.result,
        Err(DebugError::SlashCommandDirectory { .. })
    ));
    assert_eq!(outcome.stdout, "");
}

#[test]
fn debug_install_claude_command_reports_a_terminal_that_cannot_be_written() {
    let home = tempfile::tempdir().unwrap();

    let result = run_with_writers(
        &["install-claude-command"],
        &home_with(home.path(), &[]),
        UNUSED_PROXY,
        &mut BrokenPipe,
        &mut Vec::<u8>::new(),
    );

    assert!(matches!(result, Err(DebugError::Output(_))));
}

#[rstest]
fn debug_claude_replays_the_wire_corpus(#[files("fuzz/corpus/debug_wire/*")] path: PathBuf) {
    let Ok(bytes) = fs::read(&path) else {
        panic!("{}", path.display());
    };
    let _page = serde_json::from_slice::<SessionLogsPage>(&bytes);
    let _payload = serde_json::from_slice::<Option<RequestResponsePayload>>(&bytes);
}
