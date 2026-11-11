#![cfg(test)]

mod postgres;
mod support;
mod upstream_selectors;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_postgres_fixture::fixtures::http_scenarios::{SetupJournalEntry, SetupList};
use secrecy::ExposeSecret as _;
use serde_json::Value;

use support::{
    Harness, PlannedPeer, artifact_files, artifact_named, artifacts_contain, capture,
    capture_with_provider, compare_with_headers, expected_journal, journal_list_order,
    provider_request, response, response_with_headers, scenario_text, step_with_compare,
    steps_yaml, two_step_scenario,
};

use super::compare::compare_transcripts;
use super::decode_http_scenario;
use super::model::{Limits, Step, StepCompare};
use crate::case::{
    Case, Ignore, LoadedCase, Request, decode_case, load_http_scenario_cases, parse_case,
};
use crate::http_scenarios::run::StepCapture;
use crate::mock::ProviderRequest;

const FIXTURE_LIMIT_BYTES: usize = 0x0100_0000;
const OVERSIZE_BYTES: usize = 0x0100_0001;
const RELAXED_INPUT_BYTES: usize = 0x0400_0000;

const MINIMAL_SCENARIO: &str = "kind: http_scenario
name: scenario-minimal
config: |-
  model_list: []
steps:
  - name: capture
    request:
      method: GET
      path: /first
    capture:
      - name: status
        pointer: /status
";

const CAPTURE_SCENARIO: &str = "kind: http_scenario
name: scenario-capture
config: |-
  model_list: []
env: {}
steps:
  - name: capture
    request:
      method: GET
      path: /first
    capture:
      - name: status
        pointer: /status
      - name: slash
        pointer: /body/data~1id
      - name: tilde
        pointer: /body/data~0id
  - name: substitute
    request:
      method: GET
      path: /second/${capture.status}?d=${capture.slash}&t=${capture.tilde}
      headers:
        x-scenario-status: ${capture.status}
      body:
        echo: ${capture.status}
";

const SAME_STEP_SCENARIO: &str = "kind: http_scenario
name: scenario-same-step
config: |-
  model_list: []
steps:
  - name: capture
    request:
      method: GET
      path: /first/${capture.status}
    capture:
      - name: status
        pointer: /status
";

const SECRET_SCENARIO: &str = "kind: http_scenario
name: scenario-secret
config: |-
  model_list: []
env: {}
steps:
  - name: capture
    request:
      method: GET
      path: /capture
    capture:
      - name: token
        pointer: /body/value
        secret: true
  - name: echo
    request:
      method: POST
      path: /echo/${capture.token}
      body:
        echo: ${capture.token}
";

const SECRET_HEADER_SCENARIO: &str = "kind: http_scenario
name: scenario-secret-header
config: |-
  model_list: []
env: {}
steps:
  - name: capture
    request:
      method: GET
      path: /capture
    capture:
      - name: status
        pointer: /status
        secret: true
  - name: header
    request:
      method: GET
      path: /next
      headers:
        x-scenario-status: ${capture.status}
";

const MISSING_POINTER_SCENARIO: &str = "kind: http_scenario
name: scenario-missing
config: |-
  model_list: []
env: {}
steps:
  - name: capture
    request:
      method: GET
      path: /first
    capture:
      - name: status
        pointer: /missing
  - name: next
    request:
      method: GET
      path: /next
";

const SETUP_SCENARIO: &str = r#"kind: http_scenario
name: scenario-isolated
config: |-
  model_list: []
  upstream: "{{upstream}}"
  database: "{{postgres}}"
  redis: "{{redis}}"
  literal: "${capture.keep}"
env: {}
setup:
  users:
    - user_id: user-a
    - user_id: user-b
  teams:
    - team_id: team-a
  memberships:
    - user_id: user-a
      team_id: team-a
  budgets:
    - budget_id: budget-a
      user_id: user-a
      max_budget: 10
  jwt_mappings:
    - subject: subject-a
      user_id: user-a
  keys:
    - token: sk-key-a
      user_id: user-a
  managed_files:
    - unified_file_id: file-a
      content: hello
  managed_objects:
    - unified_object_id: object-a
      content: world
  vector_stores:
    - unified_resource_id: store-a
  polling:
    - name: litellm_poll_scenario
      value: '{"object":"response","status":"queued"}'
steps:
  - name: liveliness
    request:
      method: GET
      path: /health/liveliness
"#;

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn error_body(field: &str, value: &str) -> Value {
    json(&format!("{{\"error\":{{\"{field}\":\"{value}\"}}}}"))
}

#[test]
fn rejects_unknown_field() {
    let text = "kind: http_scenario\nname: n\nconfig: c\nbogus: 1\nsteps:\n  - name: s\n    request: {method: GET, path: /x}\n";
    let error = decode_http_scenario(text.as_bytes(), &Limits::default()).unwrap_err();
    assert_eq!(error.to_string(), "scenario field \"bogus\" is not allowed");
    let loaded = load_error(text);
    assert!(
        loaded.contains("scenario field \"bogus\" is not allowed"),
        "{loaded}"
    );
}

fn load_error(text: &str) -> String {
    let directory = tempfile::Builder::new()
        .prefix("scenario-case-")
        .tempdir()
        .unwrap();
    let path = directory.path().join("case.yaml");
    fs::write(&path, text).unwrap();
    load_http_scenario_cases(&[path], &Limits::default())
        .unwrap_err()
        .to_string()
}

#[test]
fn rejects_duplicate_field() {
    let text = "kind: http_scenario\nname: n\nconfig: c\nname: m\nsteps:\n  - name: s\n    request: {method: GET, path: /x}\n";
    let error = decode_http_scenario(text.as_bytes(), &Limits::default()).unwrap_err();
    assert_eq!(error.to_string(), "duplicate field \"name\"");
    drop(parse_case(Path::new("<test>"), text).unwrap_err());
}

#[test]
fn rejects_oversize_input() {
    let oversize = vec![b'a'; OVERSIZE_BYTES];
    let error = decode_http_scenario(&oversize, &Limits::default()).unwrap_err();
    assert_eq!(error.to_string(), "scenario input exceeds 16777216 bytes");
    let boundary = vec![b'a'; FIXTURE_LIMIT_BYTES];
    let limit_error = decode_http_scenario(&boundary, &Limits::default()).unwrap_err();
    assert_ne!(
        limit_error.to_string(),
        "scenario input exceeds 16777216 bytes"
    );
}

enum Boundary {
    Steps(usize),
    Users(usize),
    FixtureBytes(usize),
}

fn boundary_document(boundary: &Boundary) -> (String, Limits) {
    match boundary {
        Boundary::Steps(count) => (scenario_text(&steps_yaml(*count)), Limits::default()),
        Boundary::Users(count) => {
            let users = (0..*count).fold(String::new(), |mut text, index| {
                writeln!(text, "    - user_id: user-{index}").unwrap();
                text
            });
            let text = format!(
                "kind: http_scenario\nname: n\nconfig: c\nenv: {{}}\nsetup:\n  users:\n{users}steps:\n  - name: s\n    request: {{method: GET, path: /x}}\n"
            );
            (text, Limits::default())
        }
        Boundary::FixtureBytes(count) => {
            let content = "a".repeat(*count);
            let text = format!(
                "kind: http_scenario\nname: n\nconfig: c\nenv: {{}}\nsetup:\n  managed_files:\n    - unified_file_id: f\n      content: \"{content}\"\nsteps:\n  - name: s\n    request: {{method: GET, path: /x}}\n"
            );
            let limits = Limits {
                min_steps: 1,
                max_steps: 32,
                max_input_bytes: RELAXED_INPUT_BYTES,
                max_rows: 256,
                max_fixture_bytes: 16 * 1024 * 1024,
            };
            (text, limits)
        }
    }
}

#[rstest::rstest]
#[case::zero_steps(
    Boundary::Steps(0),
    Some("scenario must contain between 1 and 32 steps")
)]
#[case::one_step(Boundary::Steps(1), None)]
#[case::thirty_two_steps(Boundary::Steps(32), None)]
#[case::thirty_three_steps(
    Boundary::Steps(33),
    Some("scenario must contain between 1 and 32 steps")
)]
#[case::at_row_limit(Boundary::Users(256), None)]
#[case::over_row_limit(Boundary::Users(257), Some("scenario seeds more than 256 rows"))]
#[case::at_fixture_limit(Boundary::FixtureBytes(FIXTURE_LIMIT_BYTES), None)]
#[case::over_fixture_limit(
    Boundary::FixtureBytes(OVERSIZE_BYTES),
    Some("scenario fixtures exceed 16777216 bytes")
)]
fn rejects_step_and_fixture_limits(#[case] boundary: Boundary, #[case] message: Option<&str>) {
    let (text, limits) = boundary_document(&boundary);
    let outcome = decode_http_scenario(text.as_bytes(), &limits);
    match (message, outcome) {
        (Some(expected), Err(error)) => assert_eq!(error.to_string(), expected),
        (Some(expected), Ok(_scenario)) => panic!("expected {expected}"),
        (None, Err(error)) => panic!("unexpected {error}"),
        (None, Ok(_scenario)) => {}
    }
}

#[rstest::rstest]
#[case::cost_map(
    "LITELLM_LOCAL_MODEL_COST_MAP",
    "false",
    "scenario env entry \"LITELLM_LOCAL_MODEL_COST_MAP\" is reserved"
)]
#[case::master_key(
    "LITELLM_MASTER_KEY",
    "caller-key",
    "scenario env entry \"LITELLM_MASTER_KEY\" is reserved"
)]
fn rejects_reserved_env(#[case] key: &str, #[case] value: &str, #[case] message: &str) {
    let text = format!(
        "kind: http_scenario\nname: n\nconfig: c\nenv:\n  {key}: \"{value}\"\nsteps:\n  - name: s\n    request: {{method: GET, path: /x}}\n"
    );
    let error = decode_http_scenario(text.as_bytes(), &Limits::default()).unwrap_err();
    assert_eq!(error.to_string(), message);
    drop(parse_case(Path::new("<test>"), &text).unwrap_err());
}

#[test]
fn capture_substitution() {
    let mut harness = Harness::new(CAPTURE_SCENARIO);
    let first = PlannedPeer::new(vec![
        Ok(response(
            201,
            json("{\"data/id\":\"slash-value\",\"data~id\":\"tilde-value\"}"),
        )),
        Ok(response(200, json("{}"))),
    ]);
    let requests = first.requests();
    harness.push_peer(first);
    harness.push_peer(PlannedPeer::new(vec![
        Ok(response(
            201,
            json("{\"data/id\":\"slash-value\",\"data~id\":\"tilde-value\"}"),
        )),
        Ok(response(200, json("{}"))),
    ]));
    let outcome = harness.start().unwrap();
    assert!(!outcome);
    let sent = requests.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].path, "/second/201?d=slash-value&t=tilde-value");
    assert_eq!(
        sent[1].headers.get("x-scenario-status").map(String::as_str),
        Some("201")
    );
    assert_eq!(
        sent[1].body.pointer("/echo").and_then(Value::as_str),
        Some("201")
    );
    assert_same_step_reference_not_sent();
}

fn assert_same_step_reference_not_sent() {
    let mut harness = Harness::new(SAME_STEP_SCENARIO);
    let peer = PlannedPeer::new(vec![Ok(response(200, json("{}")))]);
    let requests = peer.requests();
    harness.push_peer(peer);
    harness.push_peer(PlannedPeer::new(vec![]));
    let error = harness.start().unwrap_err();
    assert!(error.to_string().contains("unresolved capture"));
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn secret_capture_redaction() {
    let mut harness = Harness::new(SECRET_SCENARIO);
    let first = secret_peer("first");
    let requests = first.requests();
    harness.push_peer(first);
    harness.push_peer(secret_peer("second"));
    let outcome = harness.start().unwrap();
    assert!(outcome);
    assert_secret_sent(&requests.lock().unwrap());
    assert_secret_redacted(harness.root.path());
}

fn secret_peer(peer: &str) -> PlannedPeer {
    let body = format!("{{\"echo\":\"sk-scenario-secret\",\"peer\":\"{peer}\"}}");
    PlannedPeer::new(vec![
        Ok(response(200, json("{\"value\":\"sk-scenario-secret\"}"))),
        Ok(response(200, json("{\"received\":\"sk-scenario-secret\"}"))),
    ])
    .with_provider(vec![
        Vec::new(),
        vec![provider_request("POST", "/v1/echo", json(&body))],
    ])
    .with_log("gateway log body sk-scenario-secret")
}

fn assert_secret_sent(requests: &[Request]) {
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/echo/sk-scenario-secret");
    assert_eq!(
        requests[1].body.pointer("/echo").and_then(Value::as_str),
        Some("sk-scenario-secret")
    );
}

fn assert_secret_redacted(root: &Path) {
    assert!(!artifacts_contain(root, "sk-scenario-secret"));
    for name in [
        "client-litellm.json",
        "provider-litellm.json",
        "diff.json",
        "litellm.log",
    ] {
        assert!(artifact_named(root, name).contains("[REDACTED]"), "{name}");
    }
}

#[test]
fn rejects_secret_capture_header() {
    let message =
        "scenario step 1 header \"x-scenario-status\" cannot use secret capture \"status\"";
    let error =
        decode_http_scenario(SECRET_HEADER_SCENARIO.as_bytes(), &Limits::default()).unwrap_err();
    assert_eq!(error.to_string(), message);
    let loaded = load_error(SECRET_HEADER_SCENARIO);
    assert!(loaded.contains(message), "{loaded}");
}

#[test]
fn missing_capture_is_error() {
    let mut harness = Harness::new(MISSING_POINTER_SCENARIO);
    let peer = PlannedPeer::new(vec![Ok(response(200, json("{}")))]);
    let requests = peer.requests();
    harness.push_peer(peer);
    harness.push_peer(PlannedPeer::new(vec![]));
    let error = harness.start().unwrap_err();
    assert_eq!(
        error.to_string(),
        "scenario step 0 capture status missing pointer /missing"
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[derive(Clone, Copy)]
enum ChangeKind {
    Status,
    ErrorType,
    ErrorParam,
    ErrorCode,
    HeaderValue,
    HeaderMissing,
    ProviderCount,
    ProviderBody,
    ProviderPath,
    ProviderHeader,
    IgnoredError,
}

struct Comparison {
    first: Vec<StepCapture>,
    second: Vec<StepCapture>,
    steps: Vec<Step>,
}

fn changed_pointer(kind: ChangeKind) -> Option<String> {
    let comparison = change_documents(kind);
    compare_transcripts(&comparison.first, &comparison.second, &comparison.steps)
        .err()
        .map(|difference| difference.pointer)
}

fn change_documents(kind: ChangeKind) -> Comparison {
    match kind {
        ChangeKind::Status => Comparison {
            first: vec![capture(200, Value::Null)],
            second: vec![capture(201, Value::Null)],
            steps: vec![step_with_compare(StepCompare::default())],
        },
        ChangeKind::ErrorType => error_documents("type", None),
        ChangeKind::ErrorParam => error_documents("param", None),
        ChangeKind::ErrorCode => error_documents("code", None),
        ChangeKind::HeaderValue => header_documents(false),
        ChangeKind::HeaderMissing => header_documents(true),
        ChangeKind::ProviderCount => provider_count_documents(),
        ChangeKind::ProviderBody => provider_documents("body"),
        ChangeKind::ProviderPath => provider_documents("path"),
        ChangeKind::ProviderHeader => provider_documents("header"),
        ChangeKind::IgnoredError => error_documents("type", Some("/error/type")),
    }
}

fn error_documents(field: &str, ignored: Option<&str>) -> Comparison {
    let compare = StepCompare {
        headers: Vec::new(),
        headers_present: Vec::new(),
        upstream_headers: Vec::new(),
        ignore: ignored
            .map(|pointer| Ignore {
                pointer: pointer.to_owned(),
                why: "the scenario ignores this pointer".to_owned(),
            })
            .into_iter()
            .collect(),
    };
    Comparison {
        first: vec![
            capture(200, Value::Null),
            capture(400, error_body(field, "first")),
        ],
        second: vec![
            capture(200, Value::Null),
            capture(400, error_body(field, "second")),
        ],
        steps: vec![step_with_compare(compare); 2],
    }
}

fn header_documents(missing: bool) -> Comparison {
    let compare = if missing {
        compare_with_headers(Vec::new(), vec!["x-test".to_owned()])
    } else {
        compare_with_headers(vec!["x-test".to_owned()], Vec::new())
    };
    let mut headers = BTreeMap::new();
    if !missing {
        headers.insert("x-test".to_owned(), "first".to_owned());
    }
    let first = StepCapture {
        response: response_with_headers(200, headers, Value::Null),
        provider: Vec::new(),
        provider_total: 0,
    };
    Comparison {
        first: vec![first],
        second: vec![capture(200, Value::Null)],
        steps: vec![step_with_compare(compare)],
    }
}

fn provider_count_documents() -> Comparison {
    let request = provider_request("POST", "/v1/echo", Value::Null);
    Comparison {
        first: vec![capture_with_provider(Value::Null, vec![request])],
        second: vec![capture(200, Value::Null)],
        steps: vec![step_with_compare(StepCompare::default())],
    }
}

fn provider_documents(change: &str) -> Comparison {
    let reference = provider_request("POST", "/v1/echo", json("{\"echo\":\"first\"}"));
    let upstream_headers = if change == "header" {
        vec!["x-upstream".to_owned()]
    } else {
        Vec::new()
    };
    let compare = StepCompare {
        headers: Vec::new(),
        headers_present: Vec::new(),
        upstream_headers,
        ignore: Vec::new(),
    };
    let rust = match change {
        "body" => provider_request("POST", "/v1/echo", json("{\"echo\":\"second\"}")),
        "path" => provider_request("POST", "/v1/other", json("{\"echo\":\"first\"}")),
        _ => ProviderRequest {
            method: "POST".to_owned(),
            path: "/v1/echo".to_owned(),
            headers: BTreeMap::from([("x-upstream".to_owned(), "second".to_owned())]),
            body: json("{\"echo\":\"first\"}"),
        },
    };
    Comparison {
        first: vec![capture_with_provider(Value::Null, vec![reference])],
        second: vec![capture_with_provider(Value::Null, vec![rust])],
        steps: vec![step_with_compare(compare)],
    }
}

#[rstest::rstest]
#[case::status(ChangeKind::Status, Some("/steps/0/status"))]
#[case::error_type(ChangeKind::ErrorType, Some("/steps/1/response/error/type"))]
#[case::error_param(ChangeKind::ErrorParam, Some("/steps/1/response/error/param"))]
#[case::error_code(ChangeKind::ErrorCode, Some("/steps/1/response/error/code"))]
#[case::header_value(ChangeKind::HeaderValue, Some("/steps/0/response/headers/x-test"))]
#[case::header_missing(ChangeKind::HeaderMissing, Some("/steps/0/response/headers/x-test"))]
#[case::provider_count(ChangeKind::ProviderCount, Some("/steps/0/provider"))]
#[case::provider_body(ChangeKind::ProviderBody, Some("/steps/0/provider/0/body"))]
#[case::provider_path(ChangeKind::ProviderPath, Some("/steps/0/provider/0"))]
#[case::provider_header(
    ChangeKind::ProviderHeader,
    Some("/steps/0/provider/0/headers/x-upstream")
)]
#[case::ignored_error(ChangeKind::IgnoredError, None)]
fn changed_state_detected(#[case] kind: ChangeKind, #[case] pointer: Option<&str>) {
    assert_eq!(changed_pointer(kind).as_deref(), pointer);
}

#[test]
fn detects_changed_peer() {
    assert_changed_provider_body();
    assert_changed_status();
    assert_changed_journal();
}

fn assert_changed_provider_body() {
    let mut harness = Harness::new(&two_step_scenario());
    let first = PlannedPeer::new(vec![Ok(response(200, Value::Null)); 2]).with_provider(vec![
        vec![provider_request(
            "POST",
            "/v1/echo",
            json("{\"peer\":\"first\"}"),
        )],
        Vec::new(),
    ]);
    let second = PlannedPeer::new(vec![Ok(response(200, Value::Null)); 2]).with_provider(vec![
        vec![provider_request(
            "POST",
            "/v1/echo",
            json("{\"peer\":\"second\"}"),
        )],
        Vec::new(),
    ]);
    harness.push_peer(first);
    harness.push_peer(second);
    let outcome = harness.start().unwrap();
    assert!(outcome);
    let diff = artifact_named(harness.root.path(), "diff.json");
    assert!(diff.contains("/steps/0/provider/0/body"));
}

fn assert_changed_status() {
    let mut harness = Harness::new(&two_step_scenario());
    harness.push_peer(PlannedPeer::new(vec![
        Ok(response(200, Value::Null)),
        Ok(response(200, Value::Null)),
    ]));
    harness.push_peer(PlannedPeer::new(vec![
        Ok(response(503, Value::Null)),
        Ok(response(200, Value::Null)),
    ]));
    let outcome = harness.start().unwrap();
    assert!(outcome);
    let diff = artifact_named(harness.root.path(), "diff.json");
    assert!(diff.contains("/steps/0/status"));
}

fn assert_changed_journal() {
    let mut harness = Harness::new(&two_step_scenario());
    let changed = vec![SetupJournalEntry {
        list: SetupList::Users,
        index: 0,
    }];
    harness.first.journal = changed;
    harness.push_peer(PlannedPeer::new(vec![
        Ok(response(200, Value::Null)),
        Ok(response(200, Value::Null)),
    ]));
    harness.push_peer(PlannedPeer::new(vec![
        Ok(response(200, Value::Null)),
        Ok(response(200, Value::Null)),
    ]));
    let error = harness.start().unwrap_err();
    assert!(error.to_string().contains("journals differ"));
}

#[test]
fn isolated_state() {
    let mut harness = Harness::new(SETUP_SCENARIO);
    harness.first.journal = expected_journal(&harness.scenario.setup);
    harness.second.journal = expected_journal(&harness.scenario.setup);
    harness.push_peer(PlannedPeer::new(vec![Ok(response(200, json("{}")))]));
    harness.push_peer(PlannedPeer::new(vec![Ok(response(200, json("{}")))]));
    let outcome = harness.start().unwrap();
    assert!(!outcome);
    assert_fixtures_isolated(&harness);
    assert_config_tokens(&harness);
    assert_polling_entries(&harness);
    assert_physical_names_absent(&harness);
}

fn assert_fixtures_isolated(harness: &Harness) {
    assert_eq!(harness.first.needs, vec![true]);
    assert_eq!(harness.second.needs, vec![true]);
    assert_eq!(harness.first.setup.as_ref(), Some(&harness.scenario.setup));
    assert_eq!(harness.second.setup.as_ref(), Some(&harness.scenario.setup));
    assert_eq!(harness.first.journal, harness.second.journal);
    assert_eq!(
        harness.first.journal,
        expected_journal(&harness.scenario.setup)
    );
    assert_ne!(
        harness.first.names.database_url.expose_secret(),
        harness.second.names.database_url.expose_secret()
    );
    assert_ne!(
        harness.first.names.redis_prefix,
        harness.second.names.redis_prefix
    );
    let order = journal_list_order(&harness.first.journal);
    assert_eq!(
        order,
        vec![
            SetupList::Users,
            SetupList::Teams,
            SetupList::Memberships,
            SetupList::Budgets,
            SetupList::JwtMappings,
            SetupList::Keys,
            SetupList::ManagedFiles,
            SetupList::ManagedObjects,
            SetupList::VectorStores,
        ]
    );
}

fn assert_config_tokens(harness: &Harness) {
    let first = &harness.factory.starts()[0];
    assert!(first.config.contains("http://upstream"));
    assert!(first.config.contains("postgres://first"));
    assert!(first.config.contains("redis://first/0"));
    assert!(first.config.contains("${capture.keep}"));
    assert!(!first.config.contains("{{"));
    let second = &harness.factory.starts()[1];
    assert!(second.config.contains("postgres://second"));
    assert_eq!(
        first
            .env
            .get("LITELLM_LOCAL_MODEL_COST_MAP")
            .map(String::as_str),
        Some("True")
    );
    assert_eq!(
        first.env.get("LITELLM_MASTER_KEY").map(String::as_str),
        Some("oracle-master")
    );
}

fn assert_polling_entries(harness: &Harness) {
    let first = &harness.first_polling.writes[0];
    let second = &harness.second_polling.writes[0];
    assert_eq!(first.key, "litellm:polling:response:litellm_poll_scenario");
    assert_eq!(first.ttl_seconds, 3600);
    assert_eq!(
        first.value,
        "{\"object\":\"response\",\"status\":\"queued\"}"
    );
    assert_ne!(first.prefix, second.prefix);
}

fn assert_physical_names_absent(harness: &Harness) {
    for (_, text) in artifact_files(harness.root.path()) {
        for name in [
            "postgres://first",
            "prefix-first",
            "postgres://second",
            "prefix-second",
        ] {
            assert!(!text.contains(name), "{name} leaked into an artifact");
        }
    }
}

enum CleanupMode {
    Success,
    Timeout,
    ProviderFailure,
    DecoderFailure,
}

fn cleanup_harness() -> Harness {
    let mut harness = Harness::new(&two_step_scenario());
    harness.first.journal = expected_journal(&harness.scenario.setup);
    harness.second.journal = expected_journal(&harness.scenario.setup);
    harness
}

fn assert_success_cleanup() {
    let mut harness = cleanup_harness();
    let first = PlannedPeer::new(vec![Ok(response(200, Value::Null)); 2]);
    let second = PlannedPeer::new(vec![Ok(response(200, Value::Null)); 2]);
    let first_stopped = first.stopped();
    let second_stopped = second.stopped();
    harness.push_peer(first);
    harness.push_peer(second);
    let outcome = harness.start().unwrap();
    assert!(!outcome);
    assert_eq!(first_stopped.load(Ordering::SeqCst), 1);
    assert_eq!(second_stopped.load(Ordering::SeqCst), 1);
    assert_eq!(harness.first.closes, 1);
    assert_eq!(harness.second.closes, 1);
}

#[derive(Clone, Copy)]
enum PeerFailure {
    Timeout,
    Provider,
}

fn failing_harness(failure: PeerFailure) -> (Harness, Arc<AtomicUsize>) {
    let mut harness = cleanup_harness();
    let first = match failure {
        PeerFailure::Timeout => PlannedPeer::new(vec![Ok(response(200, Value::Null))])
            .advancing(harness.clock.handle(), Duration::from_secs(121)),
        PeerFailure::Provider => PlannedPeer::new(vec![Err("the provider refused".to_owned())]),
    };
    let stopped = first.stopped();
    harness.push_peer(first);
    harness.push_peer(PlannedPeer::new(vec![]));
    (harness, stopped)
}

fn assert_first_peer_cleaned(harness: &Harness, stopped: &AtomicUsize) {
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
    assert_eq!(harness.first.closes, 1);
    assert_eq!(harness.second.closes, 0);
}

fn assert_timeout_cleanup() {
    let (mut harness, stopped) = failing_harness(PeerFailure::Timeout);
    let error = harness.start().unwrap_err();
    assert_eq!(error.to_string(), "scenario exceeded 120 seconds");
    assert_first_peer_cleaned(&harness, &stopped);
    assert_eq!(harness.factory.starts().len(), 1);
}

fn assert_provider_failure_cleanup() {
    let (mut harness, stopped) = failing_harness(PeerFailure::Provider);
    let error = harness.start().unwrap_err();
    assert!(error.to_string().contains("the provider refused"));
    assert_first_peer_cleaned(&harness, &stopped);
}

fn assert_decoder_failure() {
    let text = scenario_text(&steps_yaml(0));
    let error = decode_http_scenario(text.as_bytes(), &Limits::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "scenario must contain between 1 and 32 steps"
    );
}

#[rstest::rstest]
#[case::success(CleanupMode::Success)]
#[case::timeout(CleanupMode::Timeout)]
#[case::provider_failure(CleanupMode::ProviderFailure)]
#[case::decoder_failure(CleanupMode::DecoderFailure)]
fn deadline_and_cleanup(#[case] mode: CleanupMode) {
    match mode {
        CleanupMode::Success => assert_success_cleanup(),
        CleanupMode::Timeout => assert_timeout_cleanup(),
        CleanupMode::ProviderFailure => assert_provider_failure_cleanup(),
        CleanupMode::DecoderFailure => assert_decoder_failure(),
    }
}

#[test]
fn preserves_legacy_cases() {
    let legacy = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("cases/health/liveliness.yaml"),
    )
    .unwrap();
    let case = decode_case(Path::new("<legacy>"), legacy.as_bytes()).unwrap();
    let loaded = parse_case(Path::new("<legacy>"), &legacy).unwrap();
    let LoadedCase::Http(parsed) = loaded else {
        panic!("a no-kind document stayed a B001 HttpCase");
    };
    assert_eq!(case.name, parsed.name);
    assert_eq!(case.config, parsed.config);
    assert_eq!(case.request, parsed.request);
    assert_eq!(case.upstream.len(), parsed.upstream.len());
    assert_legacy_scenario_defaults(&case);
}

fn assert_legacy_scenario_defaults(case: &Case) {
    let scenario = decode_http_scenario(MINIMAL_SCENARIO.as_bytes(), &Limits::default()).unwrap();
    assert!(scenario.env.is_empty());
    assert_eq!(scenario.setup.row_count(), 0);
    assert!(scenario.setup.polling.is_empty());
    let step = &scenario.steps[0];
    assert!(!step.capture[0].secret);
    assert!(step.upstream.is_empty());
    assert!(step.compare.headers.is_empty());
    assert!(step.compare.headers_present.is_empty());
    assert!(step.compare.upstream_headers.is_empty());
    assert!(step.compare.ignore.is_empty());
    assert_eq!(case.request.path, "/health/liveliness");
}

#[test]
fn replays_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_scenarios");
    let mut entries = fs::read_dir(&corpus).unwrap();
    entries
        .try_for_each(|entry| {
            let bytes = fs::read(entry?.path())?;
            drop(decode_http_scenario(&bytes, &Limits::default()));
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(corpus_count(&corpus) > 0);
}

fn corpus_count(corpus: &Path) -> usize {
    let mut count = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        drop(entry.unwrap());
        count = count.saturating_add(1);
    }
    count
}
