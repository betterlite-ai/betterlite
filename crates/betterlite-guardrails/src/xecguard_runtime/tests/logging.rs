use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::value::RawValue;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;

use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::wire::ResponseBody;

use super::support::{
    GUARDRAIL_NAME, SAFE, UNSAFE, field, guard_at, messages_of, mock, seen, text,
};

const SECRET_SCAN: &str =
    r#"{"decision":"UNSAFE","trace_id":"trace-9","secret_fields":{"api_key":"leak"}}"#;
const LOGGED_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}],"standard_logging_object":{"id":"call-1"}}"#;
const LOGGED_WITH_INFORMATION: &str = r#"{"litellm_params":{"metadata":{"standard_logging_guardrail_information":[{"guardrail_name":"older"}]}},"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{}}"#;
const LOGGED_WITH_EMPTY_INFORMATION: &str = r#"{"litellm_params":{"metadata":{"standard_logging_guardrail_information":[]}},"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{}}"#;
const LOGGED_WITH_NULL_INFORMATION: &str = r#"{"litellm_params":{"metadata":{"standard_logging_guardrail_information":null}},"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{}}"#;
const LOGGED_WITH_STRING_INFORMATION: &str = r#"{"litellm_params":{"metadata":{"standard_logging_guardrail_information":"yes"}},"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{}}"#;
const LOGGED_WITH_DICT_INFORMATION: &str = r#"{"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{"id":"call-1","guardrail_information":{"guardrail_name":"older"}}}"#;
const LOGGED_WITH_EXISTING_ENTRY: &str = r#"{"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{"id":"call-1","guardrail_information":[{"guardrail_name":"older"}]}}"#;
const LOGGED_WITH_NON_OBJECT: &str =
    r#"{"messages":[{"role":"user","content":"hello"}],"standard_logging_object":"not-an-object"}"#;

static CLOCK_TICKS: AtomicU64 = AtomicU64::new(0);

#[test]
fn xecguard_runtime_sync_logging_hook_appends_an_entry() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let kwargs = logged(LOGGED_REQUEST);
    let (hooked, _result) = guard.logging_hook(kwargs, chat_body("answer"));
    let entries = logged_entries(&hooked);
    assert_eq!(entries.len(), 1);
    assert_eq!(text(field(&entries[0], "guardrail_status")), "success");
    assert_eq!(text(field(&entries[0], "guardrail_mode")), "logging_only");
    assert_eq!(seen(&mock).len(), 1);
}

pub(super) async fn logging_skips_existing() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_INFORMATION);
    let before = values(&kwargs);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert_eq!(values(&kwargs), before);
    assert!(seen(&mock).is_empty());
}

pub(super) async fn logging_scans_empty_information() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_EMPTY_INFORMATION);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert_eq!(logged_entries(&kwargs).len(), 1);
    assert_eq!(seen(&mock).len(), 1);
}

pub(super) async fn logging_appends_response_entry() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    let mut guard = guard_at(&mock.base);
    guard.clock = ticking_clock;
    let mut kwargs = logged(LOGGED_REQUEST);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    let entries = logged_entries(&kwargs);
    assert_eq!(entries.len(), 1);
    assert_logging_entry(&entries[0]);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(text(field(&seen[0].body, "scan_type")), "response");
    assert_eq!(messages_of(&seen[0].body).len(), 2);
}

fn assert_logging_entry(entry: &GuardrailValue) {
    let mut names = keys(entry);
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "duration",
            "end_time",
            "guardrail_mode",
            "guardrail_name",
            "guardrail_response",
            "guardrail_status",
            "masked_entity_count",
            "start_time",
        ]
    );
    assert_eq!(text(field(entry, "guardrail_name")), GUARDRAIL_NAME);
    assert_eq!(text(field(entry, "guardrail_mode")), "logging_only");
    assert_eq!(
        text(field(entry, "guardrail_status")),
        "guardrail_intervened"
    );
    assert_eq!(field(entry, "masked_entity_count"), &GuardrailValue::Null);
    assert_eq!(
        field(entry, "start_time"),
        &GuardrailValue::Decimal(1_700_000_000.0_f64)
    );
    assert_eq!(
        field(entry, "end_time"),
        &GuardrailValue::Decimal(1_700_000_001.0_f64)
    );
    assert_eq!(field(entry, "duration"), &GuardrailValue::Decimal(1.0_f64));
    assert_eq!(
        text(field(field(entry, "guardrail_response"), "decision")),
        "UNSAFE"
    );
}

pub(super) async fn logging_appends_request_entry() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_REQUEST);
    guard.async_logging_hook(&mut kwargs, &empty_body()).await;
    let entries = logged_entries(&kwargs);
    assert_eq!(entries.len(), 1);
    assert_eq!(text(field(&entries[0], "guardrail_status")), "success");
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(text(field(&seen[0].body, "scan_type")), "input");
    assert_eq!(messages_of(&seen[0].body).len(), 1);
}

pub(super) async fn logging_empty_history() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(r#"{"standard_logging_object":{}}"#);
    let before = values(&kwargs);
    guard.async_logging_hook(&mut kwargs, &empty_body()).await;
    assert_eq!(values(&kwargs), before);
    assert!(seen(&mock).is_empty());
}

pub(super) async fn logging_failed_scan() {
    let mock = mock((500, "{}"), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_REQUEST);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert!(logged_entries(&kwargs).is_empty());
    assert_eq!(seen(&mock).len(), 1);
}

pub(super) async fn logging_swallowed_failure() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_REQUEST);
    let before = values(&kwargs);
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guardrail = set_default(subscriber);
    let mut kwargs_without_object: RequestData = kwargs.clone();
    kwargs_without_object.remove("standard_logging_object");
    let before_without_object = values(&kwargs_without_object);
    guard
        .async_logging_hook(&mut kwargs_without_object, &chat_body("answer"))
        .await;
    drop(guardrail);
    assert_eq!(values(&kwargs_without_object), before_without_object);
    assert_eq!(values(&kwargs), before);
    let printed =
        String::from_utf8(sink.0.lock().expect("the captured log").clone()).expect("utf8 log");
    assert!(
        printed.contains(
            "XecGuard logging_only swallowed exception: standard_logging_object is not an object"
        ),
        "{printed}"
    );
}

pub(super) async fn logging_scans_null_information() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_NULL_INFORMATION);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert_eq!(logged_entries(&kwargs).len(), 1);
    assert_eq!(seen(&mock).len(), 1);
}

pub(super) async fn logging_skips_string_information() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_STRING_INFORMATION);
    let before = values(&kwargs);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert_eq!(values(&kwargs), before);
    assert!(seen(&mock).is_empty());
}

pub(super) async fn logging_replaces_non_list_entry() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_DICT_INFORMATION);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    let entries = logged_entries(&kwargs);
    assert_eq!(entries.len(), 1);
    assert_eq!(text(field(&entries[0], "guardrail_mode")), "logging_only");
}

pub(super) async fn logging_appends_to_existing_list() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_EXISTING_ENTRY);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    let entries = logged_entries(&kwargs);
    assert_eq!(entries.len(), 2);
    assert_eq!(text(field(&entries[0], "guardrail_name")), "older");
    assert_eq!(text(field(&entries[1], "guardrail_mode")), "logging_only");
}

pub(super) async fn logging_swallowed_non_object() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_NON_OBJECT);
    let before = values(&kwargs);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert_eq!(values(&kwargs), before);
    assert_eq!(seen(&mock).len(), 1);
}

pub(super) async fn logging_redacts_secret_fields() {
    let mock = mock((200, SECRET_SCAN), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_REQUEST);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    let entries = logged_entries(&kwargs);
    assert_eq!(entries.len(), 1);
    let response = field(&entries[0], "guardrail_response");
    let mut names = keys(response);
    names.sort_unstable();
    assert_eq!(names, ["decision", "trace_id"]);
    assert_eq!(text(field(response, "decision")), "UNSAFE");
    assert_eq!(text(field(response, "trace_id")), "trace-9");
    assert!(!format!("{:?}", values(&kwargs)).contains("leak"));
}

fn logged(json: &str) -> RequestData {
    serde_json::from_str(json).expect("the logged kwargs")
}

fn response_body(json: &str) -> ResponseBody {
    let raw: Box<RawValue> = serde_json::from_str(json).expect("the response JSON");
    ResponseBody::from_raw(raw).expect("the response body")
}

fn chat_body(text: &str) -> ResponseBody {
    response_body(&format!(
        r#"{{"choices":[{{"message":{{"role":"assistant","content":"{text}"}}}}]}}"#
    ))
}

fn empty_body() -> ResponseBody {
    response_body(r#"{"id":"response-1","choices":[]}"#)
}

fn values(kwargs: &RequestData) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    for (key, raw) in kwargs {
        let value = serde_json::from_str(raw.get()).expect("a logged value");
        fields.insert(key.clone(), value);
    }
    GuardrailValue::Object(fields)
}

fn logged_entries(kwargs: &RequestData) -> Vec<GuardrailValue> {
    let payload = values(kwargs);
    let standard = field(&payload, "standard_logging_object");
    match field(standard, "guardrail_information") {
        GuardrailValue::Array(entries) => entries.clone(),
        _ => Vec::new(),
    }
}

fn keys(value: &GuardrailValue) -> Vec<&str> {
    match value {
        GuardrailValue::Object(fields) => fields.keys().map(String::as_str).collect(),
        _ => Vec::new(),
    }
}

fn ticking_clock() -> SystemTime {
    let tick = CLOCK_TICKS.fetch_add(1, Ordering::Relaxed);
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000_u64.saturating_add(tick)))
        .expect("the clock tick")
}

#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the log lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
