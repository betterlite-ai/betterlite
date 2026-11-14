#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use betterlite_logging::private_json::files::PrivateFiles;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tempfile::TempDir;
use tokio::runtime::{Builder, Runtime};
use url::ParseError;

use super::{Dependencies, cache_directory_name, run_statusline};
use crate::statusline::cache::Clock;
use crate::statusline::credentials::Credentials;
use crate::statusline::error::StatuslineError;
use crate::statusline::fetch::error::FetchError;
use crate::statusline::fetch::{Fetched, SessionFetch};
use crate::statusline::session::Session;
use crate::statusline::{StatuslineArgs, StatuslinePorts, run};

const SESSION_BODY: &str = r#"{"router_name":"auto","last_model":"gpt-5","spend":1.5}"#;
const NOTICE: &str = " \u{b7} Savings unavailable";
const MODELS: &str = r#"{"models":[{"id":"claude-opus-4","display_name":"Opus Four"}]}"#;
const CLAUDE_ENV: [(&str, &str); 2] = [
    ("ANTHROPIC_BASE_URL", "http://gateway.test/v1/"),
    ("ANTHROPIC_API_KEY", "sk-claude"),
];
const CODEX_ENV: [(&str, &str); 2] = [
    ("OPENAI_BASE_URL", "http://gateway.test/v1"),
    ("OPENAI_API_KEY", "sk-codex"),
];

struct FixedClock;

impl Clock for FixedClock {
    fn now_millis(&self) -> i64 {
        1_700_000_000_000
    }
}

struct Call {
    base_url: String,
    key: String,
    session_id: String,
}

struct FakeFetch {
    calls: Mutex<Vec<Call>>,
    body: Option<&'static str>,
    fails: bool,
}

impl FakeFetch {
    fn answering(body: Option<&'static str>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            body,
            fails: false,
        }
    }

    fn failing() -> Self {
        Self {
            fails: true,
            ..Self::answering(None)
        }
    }

    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl SessionFetch for FakeFetch {
    fn fetch(&self, credentials: &Credentials, session_id: &str) -> Result<Fetched, FetchError> {
        self.calls.lock().unwrap().push(Call {
            base_url: credentials.base_url.clone(),
            key: credentials.api_key.expose_secret().to_owned(),
            session_id: session_id.to_owned(),
        });
        if self.fails {
            return Err(FetchError::InvalidUrl(ParseError::EmptyHost));
        }
        Ok(Fetched {
            session: self
                .body
                .and_then(|body| Session::from_json(body.as_bytes())),
            definitive: true,
        })
    }
}

struct Harness {
    root: TempDir,
    files: PrivateFiles,
    runtime: Runtime,
}

fn own_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

impl Harness {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("home")).unwrap();
        for name in ["tmp", "a", "b", "c"] {
            fs::create_dir_all(root.path().join(name)).unwrap();
        }
        Self {
            root,
            files: PrivateFiles::default(),
            runtime: Builder::new_current_thread().enable_all().build().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn run(&self, input: &str, pairs: &[(&str, &str)], fetch: &FakeFetch) -> String {
        let vars = environment(pairs);
        let home = self.path("home");
        let temp = self.path("tmp");
        run_statusline(
            input,
            &Dependencies {
                environment: &vars,
                home_dir: &home,
                temp_dir: &temp,
                owner_uid: Some(own_uid()),
                fetch,
                clock: &FixedClock,
                files: &self.files,
                runtime: &self.runtime,
            },
        )
    }

    fn transcript(&self, model: &str) -> PathBuf {
        let path = self.path("transcript.jsonl");
        fs::write(
            &path,
            format!("{{\"type\":\"assistant\",\"message\":{{\"model\":\"{model}\"}}}}\n"),
        )
        .unwrap();
        path
    }

    fn models(&self, config_dir: &Path) {
        fs::create_dir_all(config_dir.join("cache")).unwrap();
        fs::write(config_dir.join("cache/gateway-models.json"), MODELS).unwrap();
    }

    fn cache_directories(&self) -> Vec<String> {
        let mut found = Vec::new();
        for name in ["tmp", "a", "b", "c"] {
            if self
                .path(name)
                .join(format!("litellm-statusline-{}", own_uid()))
                .is_dir()
            {
                found.push(name.to_owned());
            }
        }
        found
    }
}

fn claude_payload(transcript: &Path, session_id: &str) -> String {
    format!(
        r#"{{"transcript_path":"{}","session_id":"{session_id}","model":{{"display_name":"Shown"}}}}"#,
        transcript.display()
    )
}

fn with_environment<'env>(
    base: &[(&'env str, &'env str)],
    extra: &[(&'env str, &'env str)],
) -> Vec<(&'env str, &'env str)> {
    base.iter().chain(extra).copied().collect()
}

#[rstest]
#[case::empty("")]
#[case::not_json("not json")]
#[case::array("[]")]
#[case::null("null")]
#[case::number("42")]
#[case::empty_object("{}")]
fn a_payload_that_is_not_an_object_falls_back_to_claude(#[case] input: &str) {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(SESSION_BODY));

    assert_eq!(harness.run(input, &CLAUDE_ENV, &fetch), "claude");
    assert_eq!(fetch.count(), 0);
}

#[rstest]
#[case::display_name(r#"{"model":{"display_name":"Opus"}}"#, "Opus")]
#[case::control_characters_are_stripped(r#"{"model":{"display_name":"Op\u001bus"}}"#, "Opus")]
#[case::empty_display_name(r#"{"model":{"display_name":""}}"#, "claude")]
#[case::display_name_of_the_wrong_type(r#"{"model":{"display_name":7}}"#, "claude")]
#[case::null_display_name(r#"{"model":{"display_name":null}}"#, "claude")]
#[case::model_of_the_wrong_type(r#"{"model":"Opus"}"#, "claude")]
#[case::no_model("{}", "claude")]
#[case::transcript_of_the_wrong_type(
    r#"{"transcript_path":7,"model":{"display_name":"Opus"}}"#,
    "Opus"
)]
fn without_a_served_model_the_display_name_is_shown(#[case] input: &str, #[case] shown: &str) {
    let harness = Harness::new();

    let line = harness.run(
        input,
        &CLAUDE_ENV,
        &FakeFetch::answering(Some(SESSION_BODY)),
    );

    assert_eq!(line, shown);
}

#[test]
fn a_transcript_without_a_model_shows_the_display_name() {
    let harness = Harness::new();
    let transcript = harness.path("transcript.jsonl");
    fs::write(&transcript, "{\"type\":\"user\"}\n").unwrap();
    let input = claude_payload(&transcript, "s1");

    assert_eq!(
        harness.run(
            &input,
            &CLAUDE_ENV,
            &FakeFetch::answering(Some(SESSION_BODY))
        ),
        "Shown"
    );
}

#[test]
fn the_served_model_is_labelled_without_a_session_id() {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "");
    let fetch = FakeFetch::answering(Some(SESSION_BODY));

    assert_eq!(
        harness.run(&input, &CLAUDE_ENV, &fetch),
        "Routed to: claude-opus-4"
    );
    assert_eq!(fetch.count(), 0);
}

#[rstest]
#[case::no_environment(&[])]
#[case::no_key(&[("ANTHROPIC_BASE_URL", "http://gateway.test")])]
#[case::no_base_url(&[("ANTHROPIC_API_KEY", "sk-claude")])]
#[case::empty_key(&[("ANTHROPIC_BASE_URL", "http://gateway.test"), ("ANTHROPIC_API_KEY", "")])]
#[case::only_codex_credentials(&CODEX_ENV)]
fn without_usable_credentials_no_session_is_asked_for(#[case] pairs: &[(&str, &str)]) {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "s1");
    let fetch = FakeFetch::answering(Some(SESSION_BODY));

    assert_eq!(
        harness.run(&input, pairs, &fetch),
        "Routed to: claude-opus-4"
    );
    assert_eq!(fetch.count(), 0);
}

#[test]
fn a_found_session_is_asked_for_with_the_trimmed_credentials_and_drawn() {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "s1");
    let fetch = FakeFetch::answering(Some(SESSION_BODY));

    let line = harness.run(&input, &CLAUDE_ENV, &fetch);

    assert_eq!(line, format!("Routed to: gpt-5{NOTICE}"));
    let calls = fetch.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let call = calls.first().unwrap();
    assert_eq!(call.base_url, "http://gateway.test");
    assert_eq!(call.key, "sk-claude");
    assert_eq!(call.session_id, "s1");
}

#[test]
fn the_found_session_model_is_labelled_from_the_gateway_cache() {
    let harness = Harness::new();
    harness.models(&harness.path("home/.claude"));
    let input = claude_payload(&harness.transcript("claude-sonnet-4"), "s1");
    let body = r#"{"router_name":"auto","last_model":"anthropic/claude-opus-4","spend":1.5}"#;
    let fetch = FakeFetch::answering(Some(body));

    let line = harness.run(&input, &CLAUDE_ENV, &fetch);

    assert_eq!(line, format!("Routed to: Opus Four{NOTICE}"));
}

#[test]
fn the_codex_message_labels_the_found_session_model_from_the_gateway_cache() {
    let harness = Harness::new();
    harness.models(&harness.path("home/.claude"));
    let body = r#"{"router_name":"auto","last_model":"anthropic/claude-opus-4","spend":1.5}"#;
    let fetch = FakeFetch::answering(Some(body));

    let line = harness.run(
        r#"{"hook_event_name":"Stop","session_id":"s9"}"#,
        &CODEX_ENV,
        &fetch,
    );

    assert_eq!(
        line,
        format!(r#"{{"systemMessage":"\nRouted to: Opus Four{NOTICE}"}}"#)
    );
}

#[rstest]
#[case::with_a_uid(Some(1000), "litellm-statusline-1000")]
#[case::without_a_uid(None, "litellm-statusline")]
fn the_cache_directory_is_named_the_way_litellm_names_it(
    #[case] owner_uid: Option<u32>,
    #[case] expected: &str,
) {
    assert_eq!(cache_directory_name(owner_uid), expected);
}

#[test]
fn a_missing_session_keeps_the_transcript_model() {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "s1");
    let fetch = FakeFetch::answering(None);

    assert_eq!(
        harness.run(&input, &CLAUDE_ENV, &fetch),
        "Routed to: claude-opus-4"
    );
    assert_eq!(fetch.count(), 1);
}

#[test]
fn a_failed_lookup_falls_back_to_the_display_name() {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "s1");

    assert_eq!(
        harness.run(&input, &CLAUDE_ENV, &FakeFetch::failing()),
        "Shown"
    );
}

#[test]
fn a_failed_lookup_without_a_display_name_falls_back_to_claude() {
    let harness = Harness::new();
    let transcript = harness.transcript("claude-opus-4");
    let input = format!(
        r#"{{"transcript_path":"{}","session_id":"s1"}}"#,
        transcript.display()
    );

    assert_eq!(
        harness.run(&input, &CLAUDE_ENV, &FakeFetch::failing()),
        "claude"
    );
}

#[test]
fn a_second_status_line_inside_the_ttl_is_served_from_the_cache() {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "s1");
    let fetch = FakeFetch::answering(Some(SESSION_BODY));

    let first = harness.run(&input, &CLAUDE_ENV, &fetch);
    let second = harness.run(&input, &CLAUDE_ENV, &fetch);

    assert_eq!(first, second);
    assert_eq!(fetch.count(), 1);
    let entries: Vec<_> = fs::read_dir(
        harness
            .path("tmp")
            .join(format!("litellm-statusline-{}", own_uid())),
    )
    .unwrap()
    .collect();
    assert_eq!(entries.len(), 1);
}

#[rstest]
#[case::tmpdir_first(&[("TMPDIR", "a"), ("TEMP", "b"), ("TMP", "c")], "a")]
#[case::empty_tmpdir_is_skipped(&[("TMPDIR", ""), ("TEMP", "b"), ("TMP", "c")], "b")]
#[case::temp_before_tmp(&[("TEMP", "b"), ("TMP", "c")], "b")]
#[case::tmp_last(&[("TMP", "c")], "c")]
#[case::injected_default(&[], "tmp")]
#[case::all_empty_uses_the_default(&[("TMPDIR", ""), ("TEMP", ""), ("TMP", "")], "tmp")]
fn the_cache_lives_in_the_first_non_empty_temp_directory(
    #[case] temp_vars: &[(&str, &str)],
    #[case] expected: &str,
) {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "s1");
    let located: Vec<(String, String)> = temp_vars
        .iter()
        .map(|(name, value)| {
            let shown = if value.is_empty() {
                String::new()
            } else {
                harness.path(value).display().to_string()
            };
            ((*name).to_owned(), shown)
        })
        .collect();
    let mut pairs: Vec<(&str, &str)> = CLAUDE_ENV.to_vec();
    pairs.extend(
        located
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    );

    let _line = harness.run(&input, &pairs, &FakeFetch::answering(Some(SESSION_BODY)));

    assert_eq!(harness.cache_directories(), vec![expected.to_owned()]);
}

#[rstest]
#[case::custom_directory(Some("custom"), true)]
#[case::unset_uses_the_home_directory(None, false)]
#[case::empty_uses_the_home_directory(Some(""), false)]
fn the_model_cache_is_read_from_the_config_directory(
    #[case] configured: Option<&str>,
    #[case] custom: bool,
) {
    let harness = Harness::new();
    harness.models(&harness.path(if custom { "custom" } else { "home/.claude" }));
    let input = claude_payload(&harness.transcript("claude-opus-4"), "");
    let custom_dir = harness.path("custom").display().to_string();
    let pairs: Vec<(&str, &str)> = match configured {
        Some("") => vec![("CLAUDE_CONFIG_DIR", "")],
        Some(_) => vec![("CLAUDE_CONFIG_DIR", custom_dir.as_str())],
        None => Vec::new(),
    };

    let line = harness.run(&input, &pairs, &FakeFetch::answering(None));

    assert_eq!(line, "Routed to: Opus Four");
}

#[rstest]
#[case::colored(&[("TERM", "xterm-256color")], true)]
#[case::no_term(&[], false)]
#[case::empty_term(&[("TERM", "")], false)]
#[case::dumb_term(&[("TERM", "dumb")], false)]
#[case::no_color_set(&[("TERM", "xterm"), ("NO_COLOR", "1")], false)]
#[case::no_color_set_but_empty(&[("TERM", "xterm"), ("NO_COLOR", "")], false)]
fn color_follows_term_and_no_color(#[case] extra: &[(&str, &str)], #[case] colored: bool) {
    let harness = Harness::new();
    let input = claude_payload(&harness.transcript("claude-opus-4"), "s1");
    let pairs = with_environment(&CLAUDE_ENV, extra);

    let line = harness.run(&input, &pairs, &FakeFetch::answering(Some(SESSION_BODY)));

    let plain = format!("Routed to: gpt-5{NOTICE}");
    let expected = if colored {
        format!("\x1b[1mRouted to: gpt-5\x1b[0m{NOTICE}")
    } else {
        plain
    };
    assert_eq!(line, expected);
}

#[test]
fn the_codex_stop_hook_prints_a_system_message_without_color() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(SESSION_BODY));
    let pairs = with_environment(&CODEX_ENV, &[("TERM", "xterm")]);

    let line = harness.run(
        r#"{"hook_event_name":"Stop","session_id":"s9"}"#,
        &pairs,
        &fetch,
    );

    assert_eq!(
        line,
        format!(r#"{{"systemMessage":"\nRouted to: gpt-5{NOTICE}"}}"#)
    );
    let calls = fetch.calls.lock().unwrap();
    let call = calls.first().unwrap();
    assert_eq!(call.base_url, "http://gateway.test");
    assert_eq!(call.key, "sk-codex");
    assert_eq!(call.session_id, "s9");
}

#[test]
fn the_codex_stop_hook_never_touches_the_cache() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(SESSION_BODY));
    let input = r#"{"hook_event_name":"Stop","session_id":"s9"}"#;

    let first = harness.run(input, &CODEX_ENV, &fetch);
    let second = harness.run(input, &CODEX_ENV, &fetch);

    assert_eq!(first, second);
    assert_eq!(fetch.count(), 2);
    assert!(harness.cache_directories().is_empty());
}

#[rstest]
#[case::no_session_id(r#"{"hook_event_name":"Stop"}"#, &CODEX_ENV, Some(SESSION_BODY), 0)]
#[case::empty_session_id(r#"{"hook_event_name":"Stop","session_id":""}"#, &CODEX_ENV, Some(SESSION_BODY), 0)]
#[case::no_credentials(r#"{"hook_event_name":"Stop","session_id":"s"}"#, &[], Some(SESSION_BODY), 0)]
#[case::claude_credentials_do_not_count(r#"{"hook_event_name":"Stop","session_id":"s"}"#, &CLAUDE_ENV, Some(SESSION_BODY), 0)]
#[case::no_session(r#"{"hook_event_name":"Stop","session_id":"s"}"#, &CODEX_ENV, None, 1)]
fn the_codex_stop_hook_prints_nothing_without_a_session(
    #[case] input: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] body: Option<&'static str>,
    #[case] calls: usize,
) {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(body);

    assert_eq!(harness.run(input, pairs, &fetch), "");
    assert_eq!(fetch.count(), calls);
}

#[test]
fn a_failed_codex_lookup_prints_nothing_at_all() {
    let harness = Harness::new();

    let line = harness.run(
        r#"{"hook_event_name":"Stop","session_id":"s","model":{"display_name":"Shown"}}"#,
        &CODEX_ENV,
        &FakeFetch::failing(),
    );

    assert_eq!(line, "");
}

#[rstest]
#[case::stop(r#"{"hook_event_name":"Stop"}"#, "")]
#[case::lower_case(r#"{"hook_event_name":"stop"}"#, "claude")]
#[case::trailing_space(r#"{"hook_event_name":"Stop "}"#, "claude")]
#[case::other_event(r#"{"hook_event_name":"UserPromptSubmit"}"#, "claude")]
#[case::number(r#"{"hook_event_name":1}"#, "claude")]
#[case::null(r#"{"hook_event_name":null}"#, "claude")]
#[case::absent("{}", "claude")]
fn only_the_stop_event_selects_the_codex_path(#[case] input: &str, #[case] expected: &str) {
    let harness = Harness::new();

    assert_eq!(
        harness.run(input, &[], &FakeFetch::answering(None)),
        expected
    );
}

struct Broken;

impl Read for Broken {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("no stdin"))
    }
}

impl Write for Broken {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("no stdout"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn run_with(
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    harness: &Harness,
) -> Result<(), StatuslineError> {
    let vars = BTreeMap::new();
    let home = harness.path("home");
    let temp = harness.path("tmp");
    run(
        StatuslineArgs::default(),
        &mut StatuslinePorts {
            stdin,
            stdout,
            environment: &vars,
            home_dir: &home,
            temp_dir: &temp,
        },
    )
}

#[rstest]
#[case::claude("garbage", "claude")]
#[case::claude_display_name(r#"{"model":{"display_name":"Opus"}}"#, "Opus")]
#[case::codex(r#"{"hook_event_name":"Stop"}"#, "")]
fn run_writes_the_status_line_to_stdout(#[case] input: &str, #[case] expected: &str) {
    let harness = Harness::new();
    let mut stdin = input.as_bytes();
    let mut stdout: Vec<u8> = Vec::new();

    run_with(&mut stdin, &mut stdout, &harness).unwrap();

    assert_eq!(String::from_utf8(stdout).unwrap(), expected);
}

#[test]
fn run_reports_a_stdin_that_cannot_be_read() {
    let harness = Harness::new();
    let mut stdout: Vec<u8> = Vec::new();

    let failed = run_with(&mut Broken, &mut stdout, &harness);

    assert!(matches!(failed, Err(StatuslineError::Stdin(_))));
    assert!(stdout.is_empty());
}

#[test]
fn run_reports_a_stdout_that_cannot_be_written() {
    let harness = Harness::new();
    let mut stdin = "garbage".as_bytes();

    let failed = run_with(&mut stdin, &mut Broken, &harness);

    assert!(matches!(failed, Err(StatuslineError::Stdout(_))));
}
