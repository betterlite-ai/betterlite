#![cfg(test)]

mod descriptor_service;
mod support;

use std::fs;
use std::path::Path;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_models::decisions::Json;
use indexmap::{IndexMap, IndexSet};
use rstest::rstest;
use tracing::Level;

use super::DecodedBatchInputLine;
use super::access::{self, AccessFailure};
use super::decode_batch_input_accounting;
use super::error::{BatchHttpError, BatchManagedFileError};
use super::input::BatchInputAccountingInput;
use super::ports::{BatchAccessCheck, BatchKeyGrant, BatchTeamLookup};
use super::result::BatchInputAccountingResult;
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::request_destination::RequestData;
use support::{
    AccessMethod, FakeError, Harness, RecordedFetch, Team, block_on, input_wire, managed_file_id,
    row_usage,
};

const KEY_CALLER: &str = r#"{"api_key": "sk-test"}"#;
const TEAM_CALLER: &str =
    r#"{"api_key": "sk-test", "team_id": "team-1", "models": ["all-team-models"]}"#;
const ALL_MODELS_CALLER: &str = r#"{"api_key": "sk-test", "models": ["all-team-models"]}"#;
const PROJECT_CALLER: &str =
    r#"{"api_key": "sk-test", "project_metadata": {"model_otpm_limit": {"a": 30, "b": 2}}}"#;
const EMPTY_PROJECT_CALLER: &str =
    r#"{"api_key": "sk-test", "project_metadata": {"model_otpm_limit": {}}}"#;
const MIXED_PROJECT_CALLER: &str =
    r#"{"api_key": "sk-test", "project_metadata": {"model_otpm_limit": {"a": "x", "b": 3}}}"#;
const OTPM_CALLER: &str =
    r#"{"api_key": "sk-test", "project_metadata": {"model_otpm_limit": {"gpt-4o": 3}}}"#;
const TEAM_FAILURE_DETAIL: &str =
    r#"{"error":"Batch input file model access could not be validated against the current team."}"#;

const OTPM_DESCRIPTORS: &str = r#"[{"key": "model_per_project_itpm", "value": "gpt-4o", "rate_limit": {"tokens_per_unit": 1}}, {"key": "model_per_project_otpm", "value": "gpt-4o", "rate_limit": {"requests_per_unit": 4}}, {"key": "model_per_project_otpm", "value": "model-2", "rate_limit": {"tokens_per_unit": 9, "requests_per_unit": 4}}]"#;

const MIXED_ROWS: &str = concat!(
    r#"{"custom_id": "a", "body": {"model": "gpt-4o", "prompt": "hello"}}"#,
    "\n",
    r#"{"custom_id": "b", "body": {"model": "claude-3", "messages": [{"role": "user", "content": "hi"}]}}"#,
    "\n",
    r#"{"custom_id": "c", "body": {"model": "gpt-4o", "input": ["x"], "max_tokens": 5}}"#,
);

const MESSAGES_ROW: &str =
    r#"{"body": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}}"#;
const PROMPT_ROW: &str = r#"{"body": {"model": "gpt-4o", "prompt": "hello there"}}"#;
const PROMPT_LIST_ROW: &str = r#"{"body": {"model": "gpt-4o", "prompt": ["one", 2, 3]}}"#;
const INPUT_ROW: &str = r#"{"body": {"model": "gpt-4o", "input": "hello"}}"#;
const PRIORITY_ROW: &str = r#"{"body": {"model": "gpt-4o", "prompt": "ignored", "messages": [{"role": "user", "content": "hi"}]}}"#;
const EMPTY_MESSAGES_ROW: &str =
    r#"{"body": {"model": "gpt-4o", "messages": [], "prompt": "hello"}}"#;
const NO_INPUT_ROW: &str = r#"{"body": {"model": "gpt-4o", "custom_id": "x"}}"#;
const FALSY_FIELDS_ROW: &str = r#"{"body": {"model": "gpt-4o", "messages": null, "input": ""}}"#;
const NESTED_PROMPT_ROW: &str = r#"{"body": {"model": "gpt-4o", "prompt": [[1, 2], "x"]}}"#;

const MALFORMED_ROW: &str = "not json row 123";
const NUMBER_ROW: &str = "17";
const LIST_ROW: &str = "[1,2,3]";
const SCALAR_BODY_ROW: &str = r#"{"body": "text"}"#;
const EMPTY_BODY_ROW: &str = r#"{"body": ""}"#;
const NULL_BODY_ROW: &str = r#"{"body": null}"#;
const NO_BODY_ROW: &str = r#"{"custom_id": "x"}"#;
const COUNTING_ROW: &str = r#"{"body": {"model": "gpt-4o", "prompt": "hello"}}"#;
const MANAGED_ROW: &str = r#"{"body": {"model": "smuggled", "prompt": "hello"}}"#;
const CAP_ROW: &str = r#"{"body": {"model": "gpt-4o", "prompt": "hello", "max_tokens": 5}}"#;
const BODY_ZERO_ROW: &str = r#"{"body": 0}"#;
const BODY_NUMBER_ROW: &str = r#"{"body": 5}"#;
const BODY_EMPTY_LIST_ROW: &str = r#"{"body": []}"#;
const BODY_LIST_ROW: &str = r#"{"body": [1]}"#;
const BODY_MESSAGES_OBJECT_ROW: &str = r#"{"body": {"messages": {}}}"#;

fn json_text(value: &Json) -> String {
    serde_json::to_string(value).expect("the value encodes")
}

fn json_detail(text: &str) -> Json {
    serde_json::from_str(text).expect("the detail decodes")
}

fn http_of(error: &FakeError) -> &BatchHttpError {
    error.http().expect("the failure is an HTTPException")
}

fn detail_of(error: &FakeError) -> String {
    json_text(&http_of(error).detail)
}

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn triples(shapes: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    shapes
        .iter()
        .map(|(kind, model, text)| ((*kind).to_owned(), (*model).to_owned(), (*text).to_owned()))
        .collect()
}

fn denied_check() -> BatchAccessCheck {
    BatchAccessCheck::Denied {
        message: "model not allowed".to_owned(),
    }
}

fn model_denial(model: &str, reason: &str) -> String {
    format!(
        r#"{{"error":"Batch input file references a model the caller is not authorized to use: model={model}, reason={reason}"}}"#
    )
}

fn team_harness(content: &str) -> Harness {
    let harness = Harness::new(content);
    harness.access.database = true;
    harness
        .access
        .script_team(vec![BatchTeamLookup::Loaded(Team("team-1"))]);
    harness
}

fn raw_of<'raw>(line: &DecodedBatchInputLine<'raw>) -> &'raw [u8] {
    match line {
        DecodedBatchInputLine::Entry { raw, .. } | DecodedBatchInputLine::Fallback { raw } => *raw,
    }
}

fn entry_of<'raw>(line: &DecodedBatchInputLine<'raw>) -> Option<&RequestData> {
    match line {
        DecodedBatchInputLine::Entry { entry, .. } => Some(entry),
        DecodedBatchInputLine::Fallback { .. } => None,
    }
}

#[test]
fn decode_keeps_entries_and_fallbacks() {
    let bytes = b" {\"body\": true}\r\n\nnot json\n\xef\xbb\xbf{\"body\": {}}\n[1]\n\xff\xfe\n";
    let lines =
        decode_batch_input_accounting(bytes, &ApiLimits::default()).expect("the lines decode");
    assert_eq!(lines.len(), 5);
    assert_eq!(raw_of(&lines[0]), b"{\"body\": true}");
    assert_eq!(
        entry_of(&lines[0]).and_then(|entry| entry.get("body")),
        Some(&Json::Bool(true))
    );
    assert!(entry_of(&lines[1]).is_none());
    assert_eq!(raw_of(&lines[1]), b"not json");
    assert!(entry_of(&lines[2]).is_some());
    assert!(entry_of(&lines[3]).is_none());
    assert_eq!(raw_of(&lines[3]), b"[1]");
    assert!(entry_of(&lines[4]).is_none());
    assert_eq!(raw_of(&lines[4]), b"\xff\xfe");
}

#[rstest]
#[case::json(
    ApiLimits { max_json_bytes: 4, max_event_bytes: 1024, max_items: 1024 },
    ApiLimitError::JsonTooLarge { observed: 5, limit: 4 }
)]
#[case::event(
    ApiLimits { max_json_bytes: 1024, max_event_bytes: 4, max_items: 1024 },
    ApiLimitError::EventTooLarge { observed: 5, limit: 4 }
)]
#[case::items(
    ApiLimits { max_json_bytes: 1024, max_event_bytes: 1024, max_items: 0 },
    ApiLimitError::TooManyItems { observed: 1, limit: 0 }
)]
fn decoder_enforces_limits(#[case] limits: ApiLimits, #[case] expected: ApiLimitError) {
    let error =
        decode_batch_input_accounting(b"12345", &limits).expect_err("the limit rejects the file");
    assert_eq!(error, expected);
}

#[test]
fn empty_file_accounts_nothing() {
    let harness = Harness::new(" \n\t\n");
    let result = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect("an empty file accounts");
    assert_eq!(result, BatchInputAccountingResult::default());
    assert_eq!(harness.calls.resolves().len(), 1);
    assert_eq!(harness.files.recorded().len(), 1);
    assert!(harness.counter.calls().is_empty());
    assert!(harness.access.asked().is_empty());
    assert!(harness.logs.silence());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MixedOutcome {
    Counted,
    Rejected,
}

#[derive(Debug)]
struct MixedCase {
    caller: &'static str,
    database: bool,
    deny_model: Option<&'static str>,
    asked: &'static [&'static str],
    outcome: MixedOutcome,
}

#[rstest]
#[case::key_path(MixedCase {
    caller: KEY_CALLER,
    database: false,
    deny_model: None,
    asked: &["key:gpt-4o", "key:claude-3"],
    outcome: MixedOutcome::Counted,
})]
#[case::team_path(MixedCase {
    caller: TEAM_CALLER,
    database: true,
    deny_model: None,
    asked: &["team:team-1", "team_access:gpt-4o", "member:gpt-4o", "team_access:claude-3", "member:claude-3"],
    outcome: MixedOutcome::Counted,
})]
#[case::team_path_without_a_database(MixedCase {
    caller: TEAM_CALLER,
    database: false,
    deny_model: None,
    asked: &["key:gpt-4o", "key:claude-3"],
    outcome: MixedOutcome::Counted,
})]
#[case::team_models_without_a_team(MixedCase {
    caller: ALL_MODELS_CALLER,
    database: true,
    deny_model: None,
    asked: &["key:gpt-4o", "key:claude-3"],
    outcome: MixedOutcome::Counted,
})]
#[case::key_denial(MixedCase {
    caller: KEY_CALLER,
    database: false,
    deny_model: Some("claude-3"),
    asked: &["key:gpt-4o", "key:claude-3"],
    outcome: MixedOutcome::Rejected,
})]
#[case::team_denial(MixedCase {
    caller: TEAM_CALLER,
    database: true,
    deny_model: Some("claude-3"),
    asked: &["team:team-1", "team_access:gpt-4o", "member:gpt-4o", "team_access:claude-3", "grant:claude-3"],
    outcome: MixedOutcome::Rejected,
})]
fn mixed_model_row_access(#[case] case: MixedCase) {
    let mut harness = Harness::new(MIXED_ROWS);
    harness.counter.script(&[2, 3, 1]);
    harness.calls.script(&[4, 5, 0]);
    harness.access.database = case.database;
    harness
        .access
        .script_team(vec![BatchTeamLookup::Loaded(Team("team-1"))]);
    if let Some(model) = case.deny_model {
        harness
            .access
            .key_checks
            .insert(model.to_owned(), denied_check());
        harness
            .access
            .team_checks
            .insert(model.to_owned(), denied_check());
    }
    let wire = input_wire("file-abc", Some(case.caller), "[]");
    match case.outcome {
        MixedOutcome::Counted => {
            assert_mixed_usage(&harness, &harness.run(&wire).expect("the rows account"));
        }
        MixedOutcome::Rejected => {
            assert_rejected(&harness, &wire, "claude-3", "model not allowed");
        }
    }
    assert_eq!(harness.access.asked(), labels(case.asked));
}

fn assert_mixed_usage(harness: &Harness, result: &BatchInputAccountingResult) {
    assert_eq!(result.total_tokens, 6);
    assert_eq!(result.output_tokens, 9);
    assert_eq!(result.request_count, 3);
    assert_eq!(
        result.per_model_usage,
        IndexMap::from([
            ("gpt-4o".to_owned(), row_usage(3, 4, 2)),
            ("claude-3".to_owned(), row_usage(3, 5, 1)),
        ])
    );
    assert_eq!(
        harness.counter.shapes(),
        triples(&[
            ("text", "gpt-4o", "hello"),
            ("messages", "claude-3", "user"),
            ("text", "gpt-4o", "x"),
        ])
    );
    assert!(harness.logs.silence());
}

fn assert_rejected(harness: &Harness, wire: &str, model: &str, reason: &str) {
    let error = harness
        .run(wire)
        .expect_err("the denied model rejects the file");
    assert_eq!(http_of(&error).status, 403);
    let detail = model_denial(model, reason);
    assert_eq!(detail_of(&error), detail);
    assert_eq!(
        harness.logs.lines(Level::WARN),
        vec![format!(
            "Batch rejected: caller not authorized for a model named in file-abc: {detail}"
        )]
    );
    assert!(harness.logs.lines(Level::ERROR).is_empty());
}

#[derive(Debug)]
struct TokensCase {
    row: &'static str,
    answers: &'static [usize],
    total: i64,
    shapes: &'static [(&'static str, &'static str, &'static str)],
}

#[rstest]
#[case::messages(TokensCase {
    row: MESSAGES_ROW,
    answers: &[5],
    total: 5,
    shapes: &[("messages", "gpt-4o", "user")],
})]
#[case::prompt_text(TokensCase {
    row: PROMPT_ROW,
    answers: &[4],
    total: 4,
    shapes: &[("text", "gpt-4o", "hello there")],
})]
#[case::prompt_chunks(TokensCase {
    row: PROMPT_LIST_ROW,
    answers: &[3],
    total: 5,
    shapes: &[("text", "gpt-4o", "one")],
})]
#[case::input_text(TokensCase {
    row: INPUT_ROW,
    answers: &[7],
    total: 7,
    shapes: &[("text", "gpt-4o", "hello")],
})]
#[case::messages_first(TokensCase {
    row: PRIORITY_ROW,
    answers: &[2],
    total: 2,
    shapes: &[("messages", "gpt-4o", "user")],
})]
#[case::empty_messages_fall_through(TokensCase {
    row: EMPTY_MESSAGES_ROW,
    answers: &[6],
    total: 6,
    shapes: &[("text", "gpt-4o", "hello")],
})]
#[case::no_countable_field(TokensCase {
    row: NO_INPUT_ROW,
    answers: &[],
    total: 0,
    shapes: &[],
})]
#[case::falsy_fields(TokensCase {
    row: FALSY_FIELDS_ROW,
    answers: &[],
    total: 0,
    shapes: &[],
})]
#[case::nested_chunks(TokensCase {
    row: NESTED_PROMPT_ROW,
    answers: &[4],
    total: 6,
    shapes: &[("text", "gpt-4o", "x")],
})]
fn batch_input_tokens(#[case] case: TokensCase) {
    let harness = Harness::new(case.row);
    harness.counter.script(case.answers);
    let result = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect("the row accounts");
    assert_eq!(result.total_tokens, case.total);
    assert_eq!(result.output_tokens, 0);
    assert_eq!(result.request_count, 1);
    assert_eq!(
        result.per_model_usage,
        IndexMap::from([("gpt-4o".to_owned(), row_usage(case.total, 0, 1))])
    );
    assert_eq!(harness.counter.shapes(), triples(case.shapes));
    assert!(harness.logs.silence());
}

#[derive(Debug)]
struct EstimateCase {
    descriptors: &'static str,
    caller: Option<&'static str>,
    min: Option<i64>,
}

#[rstest]
#[case::descriptor_limits(EstimateCase {
    descriptors: OTPM_DESCRIPTORS,
    caller: None,
    min: Some(9),
})]
#[case::project_metadata(EstimateCase {
    descriptors: "[]",
    caller: Some(PROJECT_CALLER),
    min: Some(2),
})]
#[case::descriptor_and_project(EstimateCase {
    descriptors: OTPM_DESCRIPTORS,
    caller: Some(PROJECT_CALLER),
    min: Some(2),
})]
#[case::no_limits(EstimateCase {
    descriptors: "[]",
    caller: None,
    min: None,
})]
#[case::empty_project_map(EstimateCase {
    descriptors: "[]",
    caller: Some(EMPTY_PROJECT_CALLER),
    min: None,
})]
#[case::non_integer_skipped(EstimateCase {
    descriptors: "[]",
    caller: Some(MIXED_PROJECT_CALLER),
    min: Some(3),
})]
fn batch_output_estimate(#[case] case: EstimateCase) {
    let mut harness = Harness::new(concat!(CAP_ROW, "\n", MALFORMED_ROW));
    harness.counter.script(&[3]);
    harness.calls.script(&[3]);
    harness.calls.floor = 7;
    let result = harness
        .run(&input_wire("file-abc", case.caller, case.descriptors))
        .expect("the rows account");
    assert_eq!(result.total_tokens, 7);
    assert_eq!(result.output_tokens, 10);
    assert_eq!(result.request_count, 2);
    assert_eq!(
        result.per_model_usage,
        IndexMap::from([("gpt-4o".to_owned(), row_usage(3, 3, 1))])
    );
    assert_eq!(harness.calls.seen_min(), vec![case.min, case.min]);
    assert!(harness.logs.silence());
}

#[derive(Debug)]
struct ManagedCase {
    caller: Option<&'static str>,
    targets: &'static str,
    asked: &'static [&'static str],
    managed: bool,
    caller_kept: bool,
}

#[rstest]
#[case::with_a_caller(ManagedCase {
    caller: Some(KEY_CALLER),
    targets: "target-model",
    asked: &["key:target-model"],
    managed: true,
    caller_kept: true,
})]
#[case::target_list(ManagedCase {
    caller: Some(KEY_CALLER),
    targets: "target-model,other-model",
    asked: &["key:target-model", "key:other-model"],
    managed: true,
    caller_kept: true,
})]
#[case::empty_targets(ManagedCase {
    caller: Some(KEY_CALLER),
    targets: "",
    asked: &["key:smuggled"],
    managed: true,
    caller_kept: true,
})]
#[case::without_a_caller(ManagedCase {
    caller: None,
    targets: "target-model",
    asked: &[],
    managed: false,
    caller_kept: false,
})]
fn managed_file_fetch_auth(#[case] case: ManagedCase) {
    let file_id = managed_file_id(case.targets);
    let harness = Harness::new(MANAGED_ROW);
    harness.counter.script(&[2]);
    let result = harness
        .run(&input_wire(&file_id, case.caller, "[]"))
        .expect("the managed file accounts");
    assert_eq!(result.total_tokens, 2);
    assert_eq!(result.output_tokens, 0);
    assert_eq!(result.request_count, 1);
    assert_eq!(
        result.per_model_usage,
        IndexMap::from([("smuggled".to_owned(), row_usage(2, 0, 1))])
    );
    assert_eq!(harness.access.asked(), labels(case.asked));
    let recorded = harness.files.recorded();
    match recorded.first().expect("the file was read") {
        RecordedFetch::Managed {
            file_id: read,
            caller,
        } => {
            assert!(case.managed);
            assert_eq!(read, &file_id);
            assert_eq!(*caller, case.caller_kept);
        }
        RecordedFetch::Provider {
            file_id: read,
            params,
            caller,
        } => {
            assert!(!case.managed);
            assert_eq!(read, &format!("{file_id}-resolved"));
            assert_eq!(*caller, case.caller_kept);
            assert_eq!(
                params.get("custom_llm_provider"),
                Some(&Json::Text("openai".to_owned()))
            );
        }
    }
    assert!(harness.logs.silence());
}

#[derive(Debug)]
struct UnsupportedCase {
    row: &'static str,
    answers: &'static [Option<usize>],
    total: i64,
    counted: usize,
    model: Option<&'static str>,
}

#[rstest]
#[case::malformed_line(UnsupportedCase {
    row: MALFORMED_ROW,
    answers: &[],
    total: 4,
    counted: 0,
    model: None,
})]
#[case::json_number_line(UnsupportedCase {
    row: NUMBER_ROW,
    answers: &[],
    total: 1,
    counted: 0,
    model: None,
})]
#[case::json_list_line(UnsupportedCase {
    row: LIST_ROW,
    answers: &[],
    total: 1,
    counted: 0,
    model: None,
})]
#[case::scalar_body(UnsupportedCase {
    row: SCALAR_BODY_ROW,
    answers: &[],
    total: 4,
    counted: 0,
    model: None,
})]
#[case::empty_string_body(UnsupportedCase {
    row: EMPTY_BODY_ROW,
    answers: &[],
    total: 0,
    counted: 0,
    model: None,
})]
#[case::null_body(UnsupportedCase {
    row: NULL_BODY_ROW,
    answers: &[],
    total: 0,
    counted: 0,
    model: None,
})]
#[case::missing_body(UnsupportedCase {
    row: NO_BODY_ROW,
    answers: &[],
    total: 0,
    counted: 0,
    model: None,
})]
#[case::numeric_body_zero(UnsupportedCase {
    row: BODY_ZERO_ROW,
    answers: &[],
    total: 0,
    counted: 0,
    model: None,
})]
#[case::numeric_body_nonzero(UnsupportedCase {
    row: BODY_NUMBER_ROW,
    answers: &[],
    total: 2,
    counted: 0,
    model: None,
})]
#[case::empty_list_body(UnsupportedCase {
    row: BODY_EMPTY_LIST_ROW,
    answers: &[],
    total: 0,
    counted: 0,
    model: None,
})]
#[case::list_body(UnsupportedCase {
    row: BODY_LIST_ROW,
    answers: &[],
    total: 3,
    counted: 0,
    model: None,
})]
#[case::object_messages_field(UnsupportedCase {
    row: BODY_MESSAGES_OBJECT_ROW,
    answers: &[],
    total: 0,
    counted: 0,
    model: None,
})]
#[case::counter_failure(UnsupportedCase {
    row: COUNTING_ROW,
    answers: &[None],
    total: 12,
    counted: 1,
    model: Some("gpt-4o"),
})]
fn unsupported_row_estimate(#[case] case: UnsupportedCase) {
    let harness = Harness::new(case.row);
    harness.counter.script_answers(case.answers);
    let result = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect("the row still accounts");
    assert_eq!(result.total_tokens, case.total);
    assert_eq!(result.output_tokens, 0);
    assert_eq!(result.request_count, 1);
    match case.model {
        Some(model) => assert_eq!(
            result.per_model_usage,
            IndexMap::from([(model.to_owned(), row_usage(case.total, 0, 1))])
        ),
        None => assert!(result.per_model_usage.is_empty()),
    }
    assert_eq!(harness.counter.calls().len(), case.counted);
    assert!(harness.logs.silence());
}

#[rstest]
#[case::default("", "openai")]
#[case::azure(r#", "custom_llm_provider": "azure""#, "azure")]
#[case::vertex(r#", "custom_llm_provider": "vertex_ai""#, "vertex_ai")]
fn the_provider_slug_is_forwarded(#[case] extra: &str, #[case] slug: &str) {
    let harness = Harness::new("");
    let wire = format!(r#"{{"file_id": "file-abc"{extra}}}"#);
    drop(harness.run(&wire).expect("the empty file accounts"));
    assert_eq!(
        harness.calls.resolves(),
        vec![("file-abc".to_owned(), slug.to_owned())]
    );
}

#[test]
fn team_lookup_failure_is_a_403() {
    let mut harness = Harness::new(COUNTING_ROW);
    harness.access.database = true;
    harness.access.script_team(vec![BatchTeamLookup::Failed]);
    let error = harness
        .run(&input_wire("file-abc", Some(TEAM_CALLER), "[]"))
        .expect_err("the lookup failure rejects the file");
    assert_eq!(http_of(&error).status, 403);
    let expected = json_detail(TEAM_FAILURE_DETAIL);
    assert_eq!(http_of(&error).detail, expected);
    assert_eq!(
        harness.logs.lines(Level::WARN),
        vec![format!(
            "Batch rejected: caller not authorized for a model named in file-abc: {}",
            json_text(&expected)
        )]
    );
    assert!(harness.logs.lines(Level::ERROR).is_empty());
}

#[test]
fn team_lookup_http_failure_passes_through() {
    let detail = json_detail(r#"{"error":"team store down"}"#);
    let mut harness = Harness::new(COUNTING_ROW);
    harness.access.database = true;
    harness
        .access
        .script_team(vec![BatchTeamLookup::Http(BatchHttpError {
            status: 500,
            detail: detail.clone(),
        })]);
    let error = harness
        .run(&input_wire("file-abc", Some(TEAM_CALLER), "[]"))
        .expect_err("the team store failure rejects the file");
    assert_eq!(http_of(&error).status, 500);
    assert_eq!(http_of(&error).detail, detail);
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Batch input file rejected for file-abc: status=500 detail={}",
            json_text(&detail)
        )]
    );
    assert!(harness.logs.lines(Level::WARN).is_empty());
}

#[test]
fn a_team_grant_lifts_a_team_denial() {
    let harness = team_harness(COUNTING_ROW);
    harness.counter.script(&[2]);
    harness
        .access
        .team_checks
        .insert("gpt-4o".to_owned(), denied_check());
    harness
        .access
        .grants
        .insert("gpt-4o".to_owned(), BatchKeyGrant::Granted);
    let result = harness
        .run(&input_wire("file-abc", Some(TEAM_CALLER), "[]"))
        .expect("the grant lifts the denial");
    assert_eq!(result.total_tokens, 2);
    assert_eq!(
        harness.access.asked(),
        labels(&[
            "team:team-1",
            "team_access:gpt-4o",
            "grant:gpt-4o",
            "member:gpt-4o"
        ])
    );
    assert!(harness.logs.silence());
}

#[rstest]
#[case::denied(denied_check(), "model not allowed")]
#[case::failed(BatchAccessCheck::Failed { message: "team lookup failed".to_owned() }, "team lookup failed")]
fn a_team_refusal_is_a_403(#[case] check: BatchAccessCheck, #[case] reason: &str) {
    let mut harness = team_harness(COUNTING_ROW);
    let checks = &mut harness.access.team_checks;
    checks.insert("gpt-4o".to_owned(), check);
    let wire = input_wire("file-abc", Some(TEAM_CALLER), "[]");
    assert_rejected(&harness, &wire, "gpt-4o", reason);
}

#[rstest]
#[case::denied(denied_check(), "model not allowed")]
#[case::failed(BatchAccessCheck::Failed { message: "member lookup failed".to_owned() }, "member lookup failed")]
fn a_member_refusal_is_a_403(#[case] check: BatchAccessCheck, #[case] reason: &str) {
    let mut harness = team_harness(COUNTING_ROW);
    let checks = &mut harness.access.member_checks;
    checks.insert("gpt-4o".to_owned(), check);
    let wire = input_wire("file-abc", Some(TEAM_CALLER), "[]");
    assert_rejected(&harness, &wire, "gpt-4o", reason);
}

// An upload whose target names decode to nothing checks the models its rows name, as LiteLLM's truthy test does
#[test]
fn empty_upload_targets_fall_back_to_row_models() {
    let input: BatchInputAccountingInput =
        serde_json::from_str(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
            .expect("the input decodes");
    let principal = input.caller.expect("the caller decodes");
    let caller =
        UserAPIKeyAuth::construct(AuthInput::Dict(principal.0)).expect("the caller builds");
    let harness = Harness::new(COUNTING_ROW);
    harness
        .access
        .key_checks
        .insert("gpt-4o".to_owned(), denied_check());
    let row_models = IndexSet::from(["gpt-4o".to_owned()]);
    let outcome = block_on(access::enforce(
        &harness.access,
        &caller,
        &row_models,
        Some(&[]),
    ));
    let AccessFailure::Http(http) = outcome.expect_err("a denied row model rejects the file")
    else {
        panic!("the refusal is an HTTPException");
    };
    assert_eq!(http.status, 403);
}

#[test]
fn a_key_http_outcome_passes_through() {
    let detail = json_detail(r#"{"error":"slow down"}"#);
    let harness = Harness::new(COUNTING_ROW);
    harness.access.key_checks.insert(
        "gpt-4o".to_owned(),
        BatchAccessCheck::Http(BatchHttpError {
            status: 429,
            detail: detail.clone(),
        }),
    );
    let error = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect_err("the key limit rejects the file");
    assert_eq!(http_of(&error).status, 429);
    assert_eq!(http_of(&error).detail, detail);
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Batch input file rejected for file-abc: status=429 detail={}",
            json_text(&detail)
        )]
    );
}

#[test]
fn access_port_failure_is_logged() {
    let mut harness = Harness::new(COUNTING_ROW);
    harness.access.failure = Some(AccessMethod::Key);
    let error = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect_err("the offline access port rejects the file");
    assert!(error.http().is_none());
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Error counting input file usage for file-abc: {error}"
        )]
    );
}

#[test]
fn non_bytes_content_names_the_class() {
    let harness = Harness::new("");
    harness.files.not_bytes("NoneType");
    let error = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect_err("the content type is rejected");
    assert_eq!(
        error.to_string(),
        "Expected bytes content from file retrieval for file-abc, got NoneType"
    );
    assert!(error.http().is_none());
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Error counting input file usage for file-abc: {error}"
        )]
    );
    assert!(harness.counter.calls().is_empty());
}

#[rstest]
#[case::logging_missing(BatchManagedFileError::LoggingMissing)]
#[case::hook_missing(BatchManagedFileError::HookMissing)]
#[case::not_file_endpoints(BatchManagedFileError::NotFileEndpoints)]
#[case::router_missing(BatchManagedFileError::RouterMissing)]
fn managed_hook_failures_keep_their_text(#[case] reason: BatchManagedFileError) {
    let harness = Harness::new(MANAGED_ROW);
    harness.files.managed_unavailable(reason);
    let file_id = managed_file_id("target-model");
    let error = harness
        .run(&input_wire(&file_id, Some(KEY_CALLER), "[]"))
        .expect_err("the missing hook rejects the file");
    assert_eq!(error.to_string(), reason.to_string());
    assert!(error.http().is_none());
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Error counting input file usage for {file_id}: {reason}"
        )]
    );
    assert!(harness.counter.calls().is_empty());
    assert!(harness.access.asked().is_empty());
}

#[rstest]
#[case::forbidden(403)]
#[case::unavailable(503)]
fn a_file_http_failure_keeps_its_status(#[case] status: u16) {
    let detail = json_detail(r#"{"error":"file read failed"}"#);
    let harness = Harness::new("");
    harness.files.http(BatchHttpError {
        status,
        detail: detail.clone(),
    });
    let error = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect_err("the file read failure rejects the batch");
    assert_eq!(http_of(&error).status, status);
    let expected = if status == 403 {
        format!(
            "Batch rejected: caller not authorized for a model named in file-abc: {}",
            json_text(&detail)
        )
    } else {
        format!(
            "Batch input file rejected for file-abc: status={status} detail={}",
            json_text(&detail)
        )
    };
    let level = if status == 403 {
        Level::WARN
    } else {
        Level::ERROR
    };
    assert_eq!(harness.logs.lines(level), vec![expected]);
}

#[test]
fn a_file_port_failure_is_logged() {
    let harness = Harness::new("");
    harness.files.fail();
    let error = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect_err("the offline files port rejects the batch");
    assert!(error.http().is_none());
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Error counting input file usage for file-abc: {error}"
        )]
    );
}

#[test]
fn an_estimate_failure_is_logged() {
    let harness = Harness::new(COUNTING_ROW);
    harness.calls.fail_next();
    let error = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect_err("the estimate failure rejects the file");
    assert!(error.http().is_none());
    assert_eq!(error.to_string(), "the scripted calls port failed");
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Error counting input file usage for file-abc: {error}"
        )]
    );
    assert!(harness.counter.calls().is_empty());
}

#[test]
fn an_oversized_file_is_logged() {
    let mut harness = Harness::new(COUNTING_ROW);
    harness.limits = ApiLimits {
        max_json_bytes: 4,
        max_event_bytes: 1024,
        max_items: 1024,
    };
    let error = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect_err("the oversized file is rejected");
    assert!(error.http().is_none());
    assert_eq!(
        error.to_string(),
        "the request JSON body is 48 bytes; the maximum is 4"
    );
    assert_eq!(
        harness.logs.lines(Level::ERROR),
        vec![format!(
            "Error counting input file usage for file-abc: {error}"
        )]
    );
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/batch_input_accounting");
    let replayed = fs::read_dir(corpus)
        .expect("the corpus exists")
        .map(|entry| fs::read(entry.expect("the entry lists").path()).expect("the seed reads"))
        .map(|bytes| {
            let lines = decode_batch_input_accounting(&bytes, &ApiLimits::default())
                .expect("the seed decodes");
            assert!(!lines.is_empty(), "the seed decodes to lines");
        })
        .count();
    assert!(replayed >= 5);
}
