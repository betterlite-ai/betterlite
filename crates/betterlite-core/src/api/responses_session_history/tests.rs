#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::api::responses_session_history::loads_session_lookup_settings;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_executor::block_on;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::cold::stored_request;
use super::decode::{SessionLogJsonError, SessionLogJsonKind, decode_session_log_json};
use super::error::SessionHistoryError;
use super::merge::{CompletionRequest, merge_session};
use super::query::{SESSION_SQL, session_messages};
use super::replay::replay_turn;
use super::{ChatSession, ColdStorageReader, SessionHistory, SessionLog, SpendLogQuery};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::bfl::poll::PollSleeper;
use crate::api::response_id_codecs::response::build_response_id;
use crate::api::responses_input_messages::{InputRequest, input_messages};
use crate::api::responses_tool_repair::value::Value;

const MODULE: &str = "betterlite_core::api::responses_session_history";
const TURN: &str = r#"[{"request_id": "resp_a", "session_id": "s1", "proxy_server_request": "{\"input\": \"hi\"}", "response": {"choices": [{"message": {"role": "assistant", "content": "a", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}}]}}]"#;
const TWO_TURNS: &str = r#"[{"session_id": "s1"}, {"session_id": "s2"}]"#;
const ASSISTANT: &str = r#"{"content": "a", "role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}], "function_call": null, "provider_specific_fields": null}"#;

struct ScriptedQuery {
    replies: Mutex<VecDeque<Vec<SessionLog>>>,
    calls: Mutex<Vec<(String, String)>>,
}

impl SpendLogQuery for ScriptedQuery {
    async fn session_logs(
        &self,
        sql: &'static str,
        request_id: &str,
    ) -> Result<Vec<SessionLog>, TableError> {
        self.calls
            .lock()
            .unwrap()
            .push((sql.to_owned(), request_id.to_owned()));
        Ok(self.replies.lock().unwrap().pop_front().unwrap_or_default())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the bucket did not answer")]
struct Unreachable;

#[derive(Default)]
struct ScriptedCold {
    objects: BTreeMap<String, Option<IndexMap<String, Json>>>,
    failing: bool,
    reads: Mutex<Vec<String>>,
}

impl ColdStorageReader for ScriptedCold {
    type Error = Unreachable;

    async fn proxy_server_request(
        &self,
        object_key: &str,
    ) -> Result<Option<IndexMap<String, Json>>, Unreachable> {
        self.reads.lock().unwrap().push(object_key.to_owned());
        if self.failing {
            return Err(Unreachable);
        }
        Ok(self.objects.get(object_key).cloned().flatten())
    }
}

#[derive(Default)]
struct RecordingSleeper(Mutex<Vec<Duration>>);

impl PollSleeper for RecordingSleeper {
    async fn sleep(&self, duration: Duration) {
        self.0.lock().unwrap().push(duration);
    }
}

#[derive(Clone, Default)]
struct Debugs(Arc<Mutex<Vec<String>>>);

struct FirstLine<'text>(&'text mut String);

impl Visit for FirstLine<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            let text = format!("{value:?}");
            self.0.push_str(text.lines().next().unwrap_or_default());
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Debugs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if event.metadata().target().starts_with(MODULE) {
            let mut text = String::new();
            event.record(&mut FirstLine(&mut text));
            self.0.lock().unwrap().push(text);
        }
    }
}

struct Fixture {
    query: Option<ScriptedQuery>,
    cold: ScriptedCold,
    cold_configured: bool,
    sleeper: RecordingSleeper,
    cache: DualLocalCache,
    tool_calls: IndexMap<String, Value>,
    disable_spend_logs: bool,
}

impl Fixture {
    fn service(&self) -> SessionHistory<'_, ScriptedQuery, ScriptedCold, RecordingSleeper> {
        SessionHistory {
            repository: self.query.as_ref(),
            cold_storage: self.cold_configured.then_some(&self.cold),
            sleeper: &self.sleeper,
            lookup: loads_session_lookup_settings(&BTreeMap::<String, String>::new()).unwrap(),
            disable_spend_logs: self.disable_spend_logs,
            limits: ApiLimits::default(),
            input_cache: &self.cache,
            tool_calls: &self.tool_calls,
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        let query = self.query.as_ref().unwrap();
        query.calls.lock().unwrap().clone()
    }

    fn reads(&self) -> Vec<String> {
        self.cold.reads.lock().unwrap().clone()
    }
}

fn fixture(replies: &[&str]) -> Fixture {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    let parsed = replies
        .iter()
        .map(|reply| serde_json::from_str(reply).unwrap())
        .collect();
    Fixture {
        query: Some(ScriptedQuery {
            replies: Mutex::new(parsed),
            calls: Mutex::default(),
        }),
        cold: ScriptedCold::default(),
        cold_configured: false,
        sleeper: RecordingSleeper::default(),
        cache: DualLocalCache::new(
            None,
            None,
            &Settings::default(),
            Arc::new(|| 0.0_f64),
            diagnostics,
        ),
        tool_calls: IndexMap::new(),
        disable_spend_logs: false,
    }
}

fn cold_fixture(failing: bool) -> Fixture {
    let mut setup = fixture(&[]);
    let objects = BTreeMap::from([
        (
            "key-1".to_owned(),
            serde_json::from_str(r#"{"input": "restored"}"#).unwrap(),
        ),
        ("key-empty".to_owned(), Some(IndexMap::new())),
        ("key-null".to_owned(), None),
    ]);
    setup.cold = ScriptedCold {
        objects,
        failing,
        reads: Mutex::default(),
    };
    setup.cold_configured = true;
    setup
}

fn plain(messages: Vec<Value>) -> Vec<Value> {
    messages
        .into_iter()
        .map(|message| {
            if let Value::Instance { object } = message {
                Value::Dict(object.attrs)
            } else {
                message
            }
        })
        .collect()
}

fn assembled(setup: &Fixture, input: &str, instructions: Option<&str>) -> Vec<Value> {
    let request = InputRequest {
        input: Some(serde_json::from_str(input).unwrap()),
        instructions: instructions.map(str::to_owned),
    };
    input_messages(&request, true, ApiLimits::default(), &setup.cache)
        .unwrap()
        .into_iter()
        .map(|message| serde_json::from_value(message).unwrap())
        .collect()
}

fn logged(columns: &str) -> SessionLog {
    serde_json::from_str(columns).unwrap()
}

#[rstest]
#[case::retries_until_rows(&["[]", "[]", TURN], false, 3_usize, 2_usize, Some("s1"))]
#[case::first_rows_stop(&[TURN, "[]"], false, 1_usize, 0_usize, Some("s1"))]
#[case::first_row_session(&[TWO_TURNS], false, 1_usize, 0_usize, Some("s1"))]
#[case::every_attempt_empty(&["[]", "[]", "[]", TURN], false, 3_usize, 2_usize, None)]
#[case::spend_logs_disabled(&["[]", TURN], true, 1_usize, 0_usize, None)]
fn session_query(
    #[case] replies: &[&str],
    #[case] disable_spend_logs: bool,
    #[case] queries: usize,
    #[case] sleeps: usize,
    #[case] session_id: Option<&str>,
) {
    let mut setup = fixture(replies);
    setup.disable_spend_logs = disable_spend_logs;
    let encoded = build_response_id(Some("openai"), Some("model-1"), "resp_a");
    let session = block_on(session_messages(&setup.service(), &encoded)).unwrap();
    assert_eq!(session.litellm_session_id.as_deref(), session_id);
    let expected_call = (SESSION_SQL.to_owned(), "resp_a".to_owned());
    assert_eq!(setup.calls(), vec![expected_call; queries]);
    let slept = setup.sleeper.0.lock().unwrap().clone();
    assert_eq!(slept, vec![Duration::from_millis(200); sleeps]);
}

#[test]
fn session_query_without_database_runs_nothing() {
    let mut setup = fixture(&[TURN]);
    setup.query = None;
    let session = block_on(session_messages(&setup.service(), "resp_a")).unwrap();
    assert_eq!(session, ChatSession::default());
    assert!(setup.sleeper.0.lock().unwrap().is_empty());
}

#[rstest]
#[case::found(&["[]", TURN], &[
    "inside get_chat_completion_message_history_for_previous_response_id",
    "decoding response id=resp_a",
    "Found the following spend logs for previous response id resp_a: [",
    "found 1 spend logs for this response id",
    "chat_completion_message_history [",
])]
#[case::missing(&[], &[
    "inside get_chat_completion_message_history_for_previous_response_id",
    "decoding response id=resp_a",
    "Found no spend logs for previous response id resp_a",
    "found 0 spend logs for this response id",
    "chat_completion_message_history []",
])]
fn session_query_debug_records(#[case] replies: &[&str], #[case] expected: &[&str]) {
    let setup = fixture(replies);
    let debugs = Debugs::default();
    let _guard = set_default(tracing_subscriber::registry().with(debugs.clone()));
    block_on(session_messages(&setup.service(), "resp_a")).unwrap();
    assert_eq!(*debugs.0.lock().unwrap(), expected);
}

#[rstest]
#[case::text_input(
    r#"{"input": "hi", "messages": [{"role": "user", "content": "old"}]}"#,
    Some(r#""hi""#),
    None
)]
#[case::instructions_lead(
    r#"{"input": "hi", "instructions": "be brief"}"#,
    Some(r#""hi""#),
    Some("be brief")
)]
#[case::empty_text_uses_messages(
    r#"{"input": "", "messages": [{"role": "user", "content": "old"}]}"#,
    Some(r#"[{"role": "user", "content": "old"}]"#),
    None
)]
#[case::empty_list_uses_messages(
    r#"{"input": [], "messages": [{"role": "user", "content": "old"}]}"#,
    Some(r#"[{"role": "user", "content": "old"}]"#),
    None
)]
#[case::dict_input_wrapped(
    r#"{"input": {"role": "user", "content": "hi"}}"#,
    Some(r#"[{"role": "user", "content": "hi"}]"#),
    None
)]
#[case::list_input(
    r#"{"input": [{"role": "user", "content": "hi"}], "messages": [{"role": "user", "content": "old"}]}"#,
    Some(r#"[{"role": "user", "content": "hi"}]"#),
    None
)]
#[case::nothing_to_replay(r#"{"input": "", "messages": [], "instructions": "x"}"#, None, None)]
#[case::empty_text_messages(r#"{"messages": "", "instructions": "x"}"#, None, None)]
#[case::zero_messages(r#"{"messages": 0, "instructions": "x"}"#, None, None)]
#[case::false_messages(r#"{"messages": false, "instructions": "x"}"#, None, None)]
#[case::empty_object_messages(r#"{"messages": {}, "instructions": "x"}"#, None, None)]
#[case::number_input(r#"{"input": 5, "instructions": "x"}"#, None, None)]
#[case::empty_request("{}", None, None)]
fn stored_turn_replay(
    #[case] request: &str,
    #[case] replayed: Option<&str>,
    #[case] instructions: Option<&str>,
) {
    let setup = fixture(&[]);
    let mut log = logged("{}");
    log.proxy_server_request = Some(Json::Text(request.to_owned()));
    let mut history = Vec::new();
    block_on(replay_turn(&setup.service(), &log, &mut history)).unwrap();
    let expected = replayed
        .map(|input| assembled(&setup, input, instructions))
        .unwrap_or_default();
    assert!(!history.is_empty() || replayed.is_none());
    assert_eq!(history, expected);
}

// LiteLLM hands any truthy messages value to the input transform, which only takes text or a list
#[rstest]
#[case::number(r#"{"messages": 1}"#)]
#[case::flag(r#"{"messages": true}"#)]
#[case::object(r#"{"messages": {"role": "user"}}"#)]
fn stored_turn_replay_rejects_truthy_messages(#[case] request: &str) {
    let setup = fixture(&[]);
    let mut log = logged("{}");
    log.proxy_server_request = Some(Json::Text(request.to_owned()));
    let outcome = block_on(replay_turn(&setup.service(), &log, &mut Vec::new()));
    assert!(matches!(outcome, Err(SessionHistoryError::Input(_))));
}

#[rstest]
#[case::choice(r#"{"choices": [{"message": {"role": "assistant", "content": "a", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}}]}"#, &[ASSISTANT])]
#[case::missing_message(r#"{"choices": [{"finish_reason": "stop"}]}"#, &[r#"{"content": null, "role": "assistant", "tool_calls": null, "function_call": null, "provider_specific_fields": null}"#])]
#[case::redacted_calls(r#"{"choices": [{"message": {"role": "assistant", "tool_calls": [{"id": "c1", "function": {"name": "f", "arguments": "redacted-by-litellm"}}, {"id": "c2", "type": "function", "function": {"name": "g", "arguments": "redacted-by-litellm "}}]}}]}"#, &[r#"{"content": null, "role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, {"id": "c2", "type": "function", "function": {"name": "g", "arguments": "redacted-by-litellm "}}], "function_call": null, "provider_specific_fields": null}"#])]
#[case::redacted_function_call(r#"{"choices": [{"message": {"role": "user", "content": "x", "tool_calls": [], "function_call": {"name": "f", "arguments": "redacted-by-litellm"}, "audio": null, "refusal": null, "provider_specific_fields": {}}}]}"#, &[r#"{"content": "x", "role": "user", "tool_calls": null, "function_call": {"name": "f", "arguments": "{}"}, "provider_specific_fields": null, "refusal": null}"#])]
#[case::kept_fields(r#"{"choices": [{"message": {"role": null, "content": "x", "function_call": {"name": "f", "arguments": "{\"a\": 1}"}, "reasoning_content": "r", "provider_specific_fields": {"p": 1}}}]}"#, &[r#"{"content": "x", "role": "assistant", "tool_calls": null, "function_call": {"name": "f", "arguments": "{\"a\": 1}"}, "provider_specific_fields": {"p": 1}, "reasoning_content": "r"}"#])]
#[case::missing_arguments(r#"{"choices": [{"message": {"content": "x", "tool_calls": [{"id": "c1", "function": {"name": "f", "arguments": null}}], "function_call": {"name": "g"}}}]}"#, &[r#"{"content": "x", "role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": ""}}], "function_call": {"name": "g", "arguments": ""}, "provider_specific_fields": null}"#])]
#[case::two_choices(r#"{"choices": [{"message": {"content": "a"}}, {"message": {"content": "b"}}]}"#, &[r#"{"content": "a", "role": "assistant", "tool_calls": null, "function_call": null, "provider_specific_fields": null}"#, r#"{"content": "b", "role": "assistant", "tool_calls": null, "function_call": null, "provider_specific_fields": null}"#])]
#[case::text_response(r#""{\"choices\": [{\"message\": {\"content\": \"a\"}}]}""#, &[])]
#[case::empty_response("{}", &[])]
#[case::missing_response("null", &[])]
fn stored_turn_replay_output(#[case] response: &str, #[case] expected: &[&str]) {
    let setup = fixture(&[]);
    let mut log = logged("{}");
    log.response = serde_json::from_str(response).unwrap();
    let mut history = Vec::new();
    block_on(replay_turn(&setup.service(), &log, &mut history)).unwrap();
    let wanted: Vec<Value> = expected
        .iter()
        .map(|message| serde_json::from_str(message).unwrap())
        .collect();
    assert_eq!(plain(history), wanted);
}

#[rstest]
#[case::unknown_role(r#"{"choices": [{"message": {"role": "bogus"}}]}"#)]
#[case::list_content(r#"{"choices": [{"message": {"content": [{"type": "text"}]}}]}"#)]
fn stored_turn_replay_rejects_invalid_response(#[case] response: &str) {
    let setup = fixture(&[]);
    let mut log = logged("{}");
    log.response = serde_json::from_str(response).unwrap();
    let outcome = block_on(replay_turn(&setup.service(), &log, &mut Vec::new()));
    assert!(matches!(outcome, Err(SessionHistoryError::Record(_))));
}

#[rstest]
#[case::without_logger(r#""{}""#, r#"{"cold_storage_object_key": "key-1"}"#, false, &[], "{}")]
#[case::null_request(r#""null""#, r#"{"cold_storage_object_key": "key-1"}"#, true, &["key-1"], r#"{"input": "restored"}"#)]
#[case::missing_request("null", r#"{"cold_storage_object_key": "key-1"}"#, true, &["key-1"], r#"{"input": "restored"}"#)]
#[case::empty_text_request(r#""""#, r#"{"cold_storage_object_key": "key-1"}"#, true, &["key-1"], r#"{"input": "restored"}"#)]
#[case::nested_marker(r#"{"input": [{"content": "a litellm_truncated b"}]}"#, r#"{"cold_storage_object_key": "key-1"}"#, true, &["key-1"], r#"{"input": "restored"}"#)]
#[case::marker_key(r#"{"input": "x", "meta": {"litellm_truncated": 1}}"#, r#"{"cold_storage_object_key": "key-1"}"#, true, &["key-1"], r#"{"input": "restored"}"#)]
#[case::complete_request(r#"{"input": "full", "n": [1, true, null]}"#, r#"{"cold_storage_object_key": "key-1"}"#, true, &[], r#"{"input": "full", "n": [1, true, null]}"#)]
#[case::metadata_text(r#""{}""#, r#""{\"cold_storage_object_key\": \"key-1\"}""#, true, &["key-1"], r#"{"input": "restored"}"#)]
#[case::key_absent(r#""{}""#, "{}", true, &[], "{}")]
#[case::metadata_absent(r#""{}""#, "null", true, &[], "{}")]
#[case::metadata_number(r#""{}""#, "5", true, &[], "{}")]
#[case::key_empty(r#""{}""#, r#"{"cold_storage_object_key": ""}"#, true, &[], "{}")]
#[case::key_number(r#""{}""#, r#"{"cold_storage_object_key": 7}"#, true, &[], "{}")]
#[case::empty_cold_result(r#"{"input": "litellm_truncated"}"#, r#"{"cold_storage_object_key": "key-empty"}"#, true, &["key-empty"], r#"{"input": "litellm_truncated"}"#)]
#[case::null_cold_result(r#""null""#, r#"{"cold_storage_object_key": "key-null"}"#, true, &["key-null"], "null")]
fn cold_payload_selection(
    #[case] request: &str,
    #[case] metadata: &str,
    #[case] logger: bool,
    #[case] reads: &[&str],
    #[case] expected: &str,
) {
    let mut setup = cold_fixture(false);
    setup.cold_configured = logger;
    let mut log = logged("{}");
    log.proxy_server_request = serde_json::from_str(request).unwrap();
    log.metadata = serde_json::from_str(metadata).unwrap();
    let debugs = Debugs::default();
    let _guard = set_default(tracing_subscriber::registry().with(debugs.clone()));
    let stored = block_on(stored_request(&setup.service(), &log)).unwrap();
    let wanted: Option<IndexMap<String, Json>> = serde_json::from_str(expected).unwrap();
    assert_eq!(stored, wanted);
    assert_eq!(setup.reads(), reads);
    let announced = debugs.0.lock().unwrap().clone();
    let read_notes = vec![
        "inside get_proxy_server_request_from_cold_storage_with_object_key..."
            .to_owned();
        reads.len()
    ];
    assert_eq!(announced, read_notes);
}

#[rstest]
#[case::malformed(r#""{""#)]
#[case::scalar(r#""5""#)]
#[case::list(r#""[1]""#)]
#[case::null(r#""null""#)]
#[case::empty(r#""""#)]
fn cold_payload_selection_logs_malformed_metadata(#[case] metadata: &str) {
    let setup = cold_fixture(false);
    let mut log = logged("{}");
    log.metadata = serde_json::from_str(metadata).unwrap();
    let debugs = Debugs::default();
    let _guard = set_default(tracing_subscriber::registry().with(debugs.clone()));
    let stored = block_on(stored_request(&setup.service(), &log)).unwrap();
    assert_eq!(stored, Some(IndexMap::new()));
    assert!(setup.reads().is_empty());
    assert_eq!(
        *debugs.0.lock().unwrap(),
        ["Failed to parse metadata from spend log to extract cold storage object key"]
    );
}

#[test]
fn cold_payload_selection_propagates_errors() {
    let setup = cold_fixture(true);
    let mut log = logged(r#"{"metadata": {"cold_storage_object_key": "key-1"}}"#);
    let failed = block_on(stored_request(&setup.service(), &log));
    assert!(matches!(
        failed,
        Err(SessionHistoryError::ColdStorage(Unreachable))
    ));
    log.proxy_server_request = Some(Json::Text("{".to_owned()));
    let malformed = block_on(stored_request(&setup.service(), &log));
    assert!(matches!(
        malformed,
        Err(SessionHistoryError::StoredRequest(
            SessionLogJsonError::Syntax {
                kind: SessionLogJsonKind::Request,
                ..
            }
        ))
    ));
    log.proxy_server_request = serde_json::from_str("[1]").unwrap();
    let listed = block_on(stored_request(&setup.service(), &log));
    assert!(matches!(
        listed,
        Err(SessionHistoryError::Column {
            column: "proxy_server_request"
        })
    ));
}

#[rstest]
#[case::request_object(b"{\"a\": 1}", SessionLogJsonKind::Request, 16_usize, Some(true))]
#[case::request_null(b"null", SessionLogJsonKind::Request, 16_usize, Some(false))]
#[case::metadata_null(b"null", SessionLogJsonKind::Metadata, 16_usize, None)]
#[case::request_list(b"[]", SessionLogJsonKind::Request, 16_usize, None)]
#[case::at_cap(b"{\"a\": 1}", SessionLogJsonKind::Metadata, 8_usize, Some(true))]
#[case::over_cap(b"{\"a\": 1}", SessionLogJsonKind::Metadata, 7_usize, None)]
fn decodes_session_log_json(
    #[case] bytes: &[u8],
    #[case] kind: SessionLogJsonKind,
    #[case] max_json_bytes: usize,
    #[case] object: Option<bool>,
) {
    let limits = ApiLimits {
        max_json_bytes,
        max_event_bytes: ApiLimits::default().max_event_bytes,
        max_items: ApiLimits::default().max_items,
    };
    let decoded = decode_session_log_json(bytes, kind, limits);
    assert_eq!(decoded.as_ref().ok().map(Option::is_some), object);
    if max_json_bytes == 7_usize {
        assert!(matches!(
            decoded,
            Err(SessionLogJsonError::TooLarge(ApiLimitError::JsonTooLarge {
                observed: 8,
                limit: 7
            }))
        ));
    }
    assert_eq!(
        SessionLogJsonKind::from_selector(2),
        SessionLogJsonKind::Request
    );
    assert_eq!(
        SessionLogJsonKind::from_selector(3),
        SessionLogJsonKind::Metadata
    );
}

#[test]
fn session_merge() {
    let setup = fixture(&[TURN]);
    let request: CompletionRequest = serde_json::from_str(
        r#"{"messages": [{"role": "tool", "tool_call_id": "", "content": "done"}]}"#,
    )
    .unwrap();
    let merged = block_on(merge_session(&setup.service(), "resp_a", &request)).unwrap();
    let mut expected = assembled(&setup, r#""hi""#, None);
    expected.push(serde_json::from_str(ASSISTANT).unwrap());
    expected.push(
        serde_json::from_str(r#"{"role": "tool", "tool_call_id": "c1", "content": "done"}"#)
            .unwrap(),
    );
    assert_eq!(plain(merged.messages), expected);
    assert_eq!(merged.litellm_trace_id.as_deref(), Some("s1"));
}

#[rstest]
#[case::no_previous_response("", 0_usize)]
#[case::no_session_rows("resp_a", 3_usize)]
fn session_merge_keeps_current_messages(
    #[case] previous_response_id: &str,
    #[case] queries: usize,
) {
    let setup = fixture(&[]);
    let request: CompletionRequest =
        serde_json::from_str(r#"{"messages": [{"role": "user", "content": "next"}]}"#).unwrap();
    let merged = block_on(merge_session(
        &setup.service(),
        previous_response_id,
        &request,
    ))
    .unwrap();
    assert_eq!(merged.messages, request.messages);
    assert_eq!(merged.litellm_trace_id, None);
    assert_eq!(setup.calls().len(), queries);
}

#[rstest]
#[case::defaults("{}", "", "")]
#[case::named(
    r#"{"model": "gpt-4o", "custom_llm_provider": "openai"}"#,
    "gpt-4o",
    "openai"
)]
fn session_merge_reports_no_messages(
    #[case] body: &str,
    #[case] model: &str,
    #[case] provider: &str,
) {
    let setup = fixture(&[]);
    let request: CompletionRequest = serde_json::from_str(body).unwrap();
    let failed = block_on(merge_session(&setup.service(), "resp_a", &request));
    let Err(SessionHistoryError::NoMessages(ErrorClass::BadRequest(details))) = failed else {
        panic!("an empty merge raises BadRequestError");
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.model.as_deref(), Some(model));
    assert_eq!(details.provider.as_deref(), Some(provider));
    assert_eq!(
        details.message,
        "Unable to create messages for completion request. This can happen when: 1) Using previous_response_id without a session database, AND 2) Input contains only function_call_output with empty or invalid call_id. Please ensure function_call_output has a valid call_id from a previous response. Original request: previous_response_id=resp_a"
    );
    assert_eq!(
        (details.body_type, details.body_param, details.body_code),
        (None, None, None)
    );
}

#[test]
fn replays_corpus() {
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/session_log_json");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .for_each(|path| {
            let bytes = fs::read(&path).unwrap();
            let (selector, stored) = bytes.split_first().unwrap();
            let kind = SessionLogJsonKind::from_selector(*selector);
            let decoded = decode_session_log_json(stored, kind, ApiLimits::default());
            let name = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(decoded.is_ok(), name.starts_with("valid-"), "{name}");
        });
}
