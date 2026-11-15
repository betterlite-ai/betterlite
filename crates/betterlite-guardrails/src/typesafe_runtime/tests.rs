#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;

use super::apply::compact;
use super::error::TypeSafeRuntimeError;
use super::messages::message_rows;
use super::{TypeSafeDefaults, TypeSafeGuard};
use crate::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, UnreachableFallback,
};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::third_party_init_b::typesafe::TypeSafeArgs;
use crate::typesafe::typesafe_config::{MaxResultChars, MinCharsToEvaluate, RelevanceThreshold};
use crate::typesafe::{DEFAULT_API_BASE, DEFAULT_JEV_TIMEOUT, DROPPED_RESULT_TEXT};

const API_KEY: &str = "test-key";
const FIRST_RESULT: &str = "結果: the first tool result";
const SECOND_RESULT: &str = "the second tool result, still needed by the assistant";
const NO_CANDIDATES_LOG: &str = "TypeSafe: no completed tool exchanges eligible for evaluation";
const ALL_RELEVANT_LOG: &str =
    "TypeSafe: all evaluated exchanges still relevant; request unchanged";

fn args(base_url: &str) -> TypeSafeArgs {
    TypeSafeArgs {
        name: "typesafe".to_owned(),
        api_base: Some(base_url.to_owned()),
        api_key: Some(SecretString::from(API_KEY)),
        model: None,
        relevance_threshold: None,
        min_chars_to_evaluate: Some(0),
        max_result_chars_in_state: None,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
        timeout: None,
        unreachable_fallback: None,
    }
}

fn no_defaults() -> TypeSafeDefaults {
    TypeSafeDefaults {
        api_base: None,
        api_key: None,
    }
}

fn base(timeout: Option<f64>) -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: Some("typesafe".to_owned()),
            supported_event_hooks: None,
            event_hook: Some(LitellmParamsMode::One("pre_call".to_owned())),
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout,
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

fn guard_at(base_url: &str, args: TypeSafeArgs) -> TypeSafeGuard {
    TypeSafeGuard::new(&args, no_defaults(), base(None), reqwest::Client::default())
        .expect("guardrail")
}

fn message(role: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        )]),
    }
}

fn call_message(call_id: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: "assistant".to_owned(),
        extra: BTreeMap::from([
            ("content".to_owned(), GuardrailValue::Null),
            (
                "tool_calls".to_owned(),
                GuardrailValue::Array(vec![tool_call(call_id)]),
            ),
        ]),
    }
}

fn tool_call(call_id: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        ("id".to_owned(), GuardrailValue::Text(call_id.to_owned())),
        (
            "type".to_owned(),
            GuardrailValue::Text("function".to_owned()),
        ),
        (
            "function".to_owned(),
            GuardrailValue::Object(BTreeMap::from([
                ("name".to_owned(), GuardrailValue::Text("lookup".to_owned())),
                (
                    "arguments".to_owned(),
                    GuardrailValue::Text("{\"q\":\"x\"}".to_owned()),
                ),
            ])),
        ),
    ]))
}

fn tool_result(call_id: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: "tool".to_owned(),
        extra: BTreeMap::from([
            (
                "tool_call_id".to_owned(),
                GuardrailValue::Text(call_id.to_owned()),
            ),
            (
                "content".to_owned(),
                GuardrailValue::Text(content.to_owned()),
            ),
        ]),
    }
}

fn two_exchanges() -> Vec<GuardrailMessage> {
    vec![
        message("user", "summarize the quarterly report"),
        call_message("call_1"),
        tool_result("call_1", FIRST_RESULT),
        call_message("call_2"),
        tool_result("call_2", SECOND_RESULT),
        message("user", "now use what you found"),
        message("assistant", "done"),
    ]
}

fn no_candidates() -> Vec<GuardrailMessage> {
    vec![message("user", "hello"), message("assistant", "hi")]
}

fn inputs(messages: Vec<GuardrailMessage>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: Some(messages),
        model: Some("gpt-4o".to_owned()),
        stream_holdback_chars: None,
    }
}

fn request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text("call-1".to_owned()),
    )])
}

fn records(request_data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata") else {
        return Vec::new();
    };
    let Some(GuardrailValue::Array(entries)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        return Vec::new();
    };
    entries.clone()
}

fn field<'value>(record: &'value GuardrailValue, key: &str) -> Option<&'value GuardrailValue> {
    match record {
        GuardrailValue::Object(fields) => fields.get(key),
        _ => None,
    }
}

fn text_field(record: &GuardrailValue, key: &str) -> Option<String> {
    if let Some(GuardrailValue::Text(text)) = field(record, key) {
        return Some(text.clone());
    }
    None
}

fn object(entries: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn status_of(record: &GuardrailValue) -> String {
    text_field(record, "guardrail_status").unwrap_or_default()
}

fn dropped_once(original: &[GuardrailMessage], index: usize) -> Vec<GuardrailMessage> {
    let mut expected = original.to_vec();
    expected[index].extra.insert(
        "content".to_owned(),
        GuardrailValue::Text(DROPPED_RESULT_TEXT.to_owned()),
    );
    expected
}

fn first_result_removed() -> i64 {
    code_points(FIRST_RESULT).saturating_sub(code_points(DROPPED_RESULT_TEXT))
}

fn success_counts(removed: i64, dropped: i64, evaluated: i64) -> GuardrailValue {
    object(vec![
        ("chars_removed", GuardrailValue::SignedInteger(removed)),
        ("exchanges_dropped", GuardrailValue::SignedInteger(dropped)),
        (
            "exchanges_evaluated",
            GuardrailValue::SignedInteger(evaluated),
        ),
        ("model", GuardrailValue::Text("jev-latest".to_owned())),
    ])
}

fn unavailable_object() -> GuardrailValue {
    object(vec![
        (
            "error",
            GuardrailValue::Text(
                "TypeSafe evaluation unavailable; request forwarded uncompacted".to_owned(),
            ),
        ),
        ("model", GuardrailValue::Text("jev-latest".to_owned())),
    ])
}

fn recorded_duration(record: &GuardrailValue) -> Option<f64> {
    if let Some(GuardrailValue::Decimal(value)) = field(record, "duration") {
        return Some(*value);
    }
    None
}

fn reply(answers: &[(&str, f64)]) -> String {
    let entries: Vec<String> = answers
        .iter()
        .map(|(id, noul)| format!(r#""{id}":{{"type":"noul","noul":{noul}}}"#))
        .collect();
    format!(r#"{{"answers":{{{}}}}}"#, entries.join(","))
}

fn question_ids(body: &str) -> Vec<String> {
    let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(body) else {
        return Vec::new();
    };
    let Some(Json::Object(questions)) = fields.get("questions") else {
        return Vec::new();
    };
    questions.keys().cloned().collect()
}

fn code_points(text: &str) -> i64 {
    i64::try_from(text.chars().count()).expect("the length fits")
}

async fn apply_request(
    guard: &TypeSafeGuard,
    inputs: GuardrailApiInputs,
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Result<GuardrailApiInputs, GuardrailFailure> {
    guard
        .apply_guardrail(inputs, data, GuardrailInputType::Request, None)
        .await
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

impl Capture {
    fn push(&self, body: String) {
        self.0.lock().expect("request capture").push(body);
    }

    fn count(&self) -> usize {
        self.0.lock().expect("request capture").len()
    }

    fn first(&self) -> String {
        let bodies = self.0.lock().expect("request capture");
        bodies.first().cloned().unwrap_or_default()
    }
}

async fn serve(status: StatusCode, body: String) -> (String, Capture, JoinSet<io::Result<()>>) {
    let requests = Capture::default();
    let recorder = requests.clone();
    let router = Router::new().route(
        "/v1/systemone",
        post(move |request_body: String| {
            let answer = body.clone();
            let requests = recorder.clone();
            async move {
                requests.push(request_body);
                (status, answer)
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (format!("http://{address}"), requests, server)
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn log_subscriber(
    level: tracing::Level,
    captured: Captured,
) -> impl tracing::Subscriber + Send + Sync {
    fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(move || captured.clone())
        .finish()
}

fn logs(captured: &Captured) -> String {
    let bytes = captured.0.lock().expect("logs").clone();
    String::from_utf8(bytes).expect("logs are utf8")
}

static TICKS: AtomicUsize = AtomicUsize::new(0);

fn ticking_now() -> SystemTime {
    let tick = TICKS.fetch_add(1, Ordering::SeqCst);
    let seconds = 1_700_000_000_u64.saturating_add(u64::try_from(tick).unwrap_or(u64::MAX));
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("ticking time")
}

#[test]
fn new_uses_litellm_defaults_when_only_the_key_is_configured() {
    let args = TypeSafeArgs {
        api_base: None,
        min_chars_to_evaluate: None,
        ..args("http://unused")
    };
    let guard = TypeSafeGuard::new(&args, no_defaults(), base(None), reqwest::Client::default())
        .expect("guard");
    assert_eq!(guard.runtime.settings.api_base, DEFAULT_API_BASE);
    assert_eq!(guard.runtime.settings.model, "jev-latest");
    assert_eq!(
        guard.runtime.settings.relevance_threshold,
        RelevanceThreshold(0.2)
    );
    assert_eq!(
        guard.runtime.settings.min_chars_to_evaluate,
        MinCharsToEvaluate(200)
    );
    assert_eq!(
        guard.runtime.settings.max_result_chars_in_state,
        MaxResultChars(4000)
    );
    assert_eq!(
        guard.runtime.settings.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
    assert_eq!(guard.runtime.timeout, DEFAULT_JEV_TIMEOUT);
    assert_eq!(guard.runtime.api_key.expose_secret(), API_KEY);
    assert_eq!(
        guard.base.settings.guardrail_name.as_deref(),
        Some("typesafe")
    );
}

#[test]
fn new_prefers_the_arguments_over_the_resolved_defaults() {
    let args = TypeSafeArgs {
        api_base: Some("https://args.example/".to_owned()),
        api_key: Some(SecretString::from("arg-key")),
        model: Some("arg-model".to_owned()),
        relevance_threshold: Some(0.5),
        min_chars_to_evaluate: Some(7),
        max_result_chars_in_state: Some(99),
        unreachable_fallback: Some("fail_closed".to_owned()),
        ..args("http://unused")
    };
    let defaults = TypeSafeDefaults {
        api_base: Some("https://defaults.example".to_owned()),
        api_key: Some(SecretString::from("default-key")),
    };
    let guard =
        TypeSafeGuard::new(&args, defaults, base(None), reqwest::Client::default()).expect("guard");
    assert_eq!(guard.runtime.settings.api_base, "https://args.example");
    assert_eq!(guard.runtime.api_key.expose_secret(), "arg-key");
    assert_eq!(guard.runtime.settings.model, "arg-model");
    assert_eq!(
        guard.runtime.settings.relevance_threshold,
        RelevanceThreshold(0.5)
    );
    assert_eq!(
        guard.runtime.settings.min_chars_to_evaluate,
        MinCharsToEvaluate(7)
    );
    assert_eq!(
        guard.runtime.settings.max_result_chars_in_state,
        MaxResultChars(99)
    );
    assert_eq!(
        guard.runtime.settings.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}

#[test]
fn new_falls_back_to_the_resolved_defaults() {
    let args = TypeSafeArgs {
        api_base: None,
        api_key: None,
        ..args("http://unused")
    };
    let defaults = TypeSafeDefaults {
        api_base: Some("https://defaults.example/".to_owned()),
        api_key: Some(SecretString::from("default-key")),
    };
    let guard =
        TypeSafeGuard::new(&args, defaults, base(None), reqwest::Client::default()).expect("guard");
    assert_eq!(guard.runtime.settings.api_base, "https://defaults.example");
    assert_eq!(guard.runtime.api_key.expose_secret(), "default-key");
}

#[rstest]
#[case::absent(None, None)]
#[case::empty_argument(Some(""), Some("default-key"))]
#[case::empty_default(None, Some(""))]
#[case::both_empty(Some(""), Some(""))]
#[test]
fn new_requires_an_api_key(#[case] argument: Option<&str>, #[case] default: Option<&str>) {
    let args = TypeSafeArgs {
        api_key: argument.map(SecretString::from),
        ..args("http://unused")
    };
    let defaults = TypeSafeDefaults {
        api_base: None,
        api_key: default.map(SecretString::from),
    };
    let error = TypeSafeGuard::new(&args, defaults, base(None), reqwest::Client::default())
        .expect_err("a key is required");
    assert_eq!(
        error.to_string(),
        "TypeSafe guardrail requires an API key. Set `api_key` in the guardrail config or the TYPESAFE_API_KEY env var."
    );
}

#[test]
fn new_keeps_a_zero_relevance_threshold_and_zero_min_chars() {
    let args = TypeSafeArgs {
        relevance_threshold: Some(0.0),
        min_chars_to_evaluate: Some(0),
        ..args("http://unused")
    };
    let guard = TypeSafeGuard::new(&args, no_defaults(), base(None), reqwest::Client::default())
        .expect("guardrail");
    assert_eq!(
        guard.runtime.settings.relevance_threshold,
        RelevanceThreshold(0.0)
    );
    assert_eq!(
        guard.runtime.settings.min_chars_to_evaluate,
        MinCharsToEvaluate(0)
    );
}

#[rstest]
#[case::exact(Some("fail_closed"), UnreachableFallback::FailClosed)]
#[case::fail_open(Some("fail_open"), UnreachableFallback::FailOpen)]
#[case::case_mismatch(Some("Fail_Closed"), UnreachableFallback::FailOpen)]
#[case::blank(Some(""), UnreachableFallback::FailOpen)]
#[case::absent(None, UnreachableFallback::FailOpen)]
#[test]
fn new_reads_only_the_exact_fail_closed_text(
    #[case] fallback: Option<&str>,
    #[case] expected: UnreachableFallback,
) {
    let args = TypeSafeArgs {
        unreachable_fallback: fallback.map(ToOwned::to_owned),
        ..args("http://unused")
    };
    let guard = TypeSafeGuard::new(&args, no_defaults(), base(None), reqwest::Client::default())
        .expect("guardrail");
    assert_eq!(guard.runtime.settings.unreachable_fallback, expected);
}

#[rstest]
#[case::default_timeout(None, DEFAULT_JEV_TIMEOUT)]
#[case::configured(Some(2.5), Duration::from_millis(2_500))]
#[test]
fn new_takes_the_base_timeout_or_the_litellm_default(
    #[case] timeout: Option<f64>,
    #[case] expected: Duration,
) {
    let guard = TypeSafeGuard::new(
        &args("http://unused"),
        no_defaults(),
        base(timeout),
        reqwest::Client::default(),
    )
    .expect("guardrail");
    assert_eq!(guard.runtime.timeout, expected);
}

#[rstest]
#[case::negative(-1.0)]
#[case::not_a_number(f64::NAN)]
#[case::huge(f64::MAX)]
#[test]
fn new_rejects_a_timeout_that_is_not_a_duration(#[case] seconds: f64) {
    let error = TypeSafeGuard::new(
        &args("http://unused"),
        no_defaults(),
        base(Some(seconds)),
        reqwest::Client::default(),
    )
    .expect_err("the timeout is not a duration");
    assert!(matches!(error, TypeSafeRuntimeError::Timeout { .. }));
}

#[rstest]
#[case::threshold(
    Some(1.5),
    None,
    None,
    "relevance_threshold must be between 0.0 and 1.0, got 1.5"
)]
#[case::negative_min_chars(
    None,
    Some(-1),
    None,
    "min_chars_to_evaluate must be at least 0, got -1"
)]
#[case::zero_max_result(None, None, Some(0), "max_result_chars_in_state must be at least 1")]
#[case::negative_max_result(
    None,
    None,
    Some(-2),
    "max_result_chars_in_state must be at least 1"
)]
#[test]
fn new_rejects_compaction_tuning_outside_the_litellm_bounds(
    #[case] threshold: Option<f64>,
    #[case] min_chars: Option<i64>,
    #[case] max_result: Option<i64>,
    #[case] message: &str,
) {
    let args = TypeSafeArgs {
        relevance_threshold: threshold,
        min_chars_to_evaluate: min_chars,
        max_result_chars_in_state: max_result,
        ..args("http://unused")
    };
    let error = TypeSafeGuard::new(&args, no_defaults(), base(None), reqwest::Client::default())
        .expect_err("the tuning is out of range");
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::response_input(
    GuardrailInputType::Response,
    two_exchanges(),
    reply(&[("e0", 0.0), ("e1", 0.0)]),
    false
)]
#[case::empty_messages(
    GuardrailInputType::Request,
    Vec::new(),
    reply(&[("e0", 0.0)]),
    false
)]
#[case::no_candidates(
    GuardrailInputType::Request,
    no_candidates(),
    reply(&[("e0", 0.0)]),
    false
)]
#[case::all_relevant(
    GuardrailInputType::Request,
    two_exchanges(),
    reply(&[("e0", 1.0), ("e1", 0.9)]),
    true
)]
#[case::missing_answers(GuardrailInputType::Request, two_exchanges(), reply(&[]), true)]
#[case::at_threshold(
    GuardrailInputType::Request,
    two_exchanges(),
    reply(&[("e0", 0.2), ("e1", 0.2)]),
    true
)]
#[tokio::test]
async fn typesafe_runtime_cases(
    #[case] input_type: GuardrailInputType,
    #[case] messages: Vec<GuardrailMessage>,
    #[case] reply_body: String,
    #[case] expects_call: bool,
) {
    let (base_url, requests, mut server) = serve(StatusCode::OK, reply_body).await;
    let guard = guard_at(&base_url, args(&base_url));
    let original = inputs(messages);
    let mut data = request_data();
    let returned = guard
        .apply_guardrail(original.clone(), &mut data, input_type, None)
        .await
        .expect("apply_guardrail");
    assert_eq!(returned, original);
    assert_eq!(requests.count(), usize::from(expects_call));
    assert!(records(&data).is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn one_exchange_is_dropped_and_the_kept_one_is_untouched() {
    let (base_url, requests, mut server) =
        serve(StatusCode::OK, reply(&[("e0", 0.0), ("e1", 0.9)])).await;
    let guard = guard_at(&base_url, args(&base_url));
    let original = inputs(two_exchanges());
    let mut data = request_data();
    let returned = apply_request(&guard, original.clone(), &mut data)
        .await
        .expect("compaction");
    assert_eq!(
        returned.structured_messages,
        Some(dropped_once(&two_exchanges(), 2))
    );
    assert_eq!(returned.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        question_ids(&requests.first()),
        ["e0".to_owned(), "e1".to_owned()]
    );
    let record = &records(&data)[0];
    assert_eq!(status_of(record), "success");
    assert_eq!(
        text_field(record, "guardrail_provider").as_deref(),
        Some("typesafe")
    );
    assert_eq!(
        text_field(record, "guardrail_mode").as_deref(),
        Some("pre_call")
    );
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&success_counts(first_result_removed(), 1, 2))
    );
    server.shutdown().await;
}

#[tokio::test]
async fn an_exchange_without_an_answer_is_kept() {
    let (base_url, _requests, mut server) = serve(StatusCode::OK, reply(&[("e0", 0.0)])).await;
    let guard = guard_at(&base_url, args(&base_url));
    let original = two_exchanges();
    let mut data = request_data();
    let returned = apply_request(&guard, inputs(original.clone()), &mut data)
        .await
        .expect("compaction");
    assert_eq!(
        returned.structured_messages,
        Some(dropped_once(&original, 2))
    );
    let record = &records(&data)[0];
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&success_counts(first_result_removed(), 1, 2))
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_failed_call_is_recorded_and_forwarded_under_fail_open() {
    let (base_url, _requests, mut server) =
        serve(StatusCode::INTERNAL_SERVER_ERROR, "down".to_owned()).await;
    let mut guard = guard_at(&base_url, args(&base_url));
    guard.clock = ticking_now;
    let original = inputs(two_exchanges());
    let mut data = request_data();
    let returned = apply_request(&guard, original.clone(), &mut data)
        .await
        .expect("fail_open forwards the request");
    assert_eq!(returned, original);
    let record = &records(&data)[0];
    assert_eq!(status_of(record), "guardrail_failed_to_respond");
    assert_eq!(
        text_field(record, "guardrail_provider").as_deref(),
        Some("typesafe")
    );
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&unavailable_object())
    );
    assert_eq!(
        field(record, "start_time"),
        Some(&GuardrailValue::Decimal(1_700_000_000.0))
    );
    assert_eq!(
        field(record, "end_time"),
        Some(&GuardrailValue::Decimal(1_700_000_001.0))
    );
    let duration = recorded_duration(record).expect("the record carries a duration");
    assert!((0.0..1.0).contains(&duration));
    server.shutdown().await;
}

#[tokio::test]
async fn a_failed_call_answers_502_under_fail_closed() {
    let (base_url, _requests, mut server) =
        serve(StatusCode::INTERNAL_SERVER_ERROR, "down".to_owned()).await;
    let args = TypeSafeArgs {
        unreachable_fallback: Some("fail_closed".to_owned()),
        ..args(&base_url)
    };
    let guard = guard_at(&base_url, args);
    let mut data = request_data();
    let failure = apply_request(&guard, inputs(two_exchanges()), &mut data)
        .await
        .expect_err("fail_closed answers the failure");
    assert_eq!(
        failure,
        GuardrailFailure::HttpObject {
            status: 502,
            detail: IndexMap::from([(
                "error".to_owned(),
                GuardrailValue::Text("TypeSafe evaluation service returned an error".to_owned()),
            )]),
        }
    );
    assert!(records(&data).is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn a_message_that_is_not_an_object_leaves_the_request_unchanged() {
    let (base_url, requests, mut server) = serve(StatusCode::OK, reply(&[("e0", 0.0)])).await;
    let guard = guard_at(&base_url, args(&base_url));
    let typed = two_exchanges();
    let mut rows = message_rows(&typed);
    rows.push(Json::Text("not a message".to_owned()));
    let original = inputs(typed);
    let mut data = request_data();
    let returned = compact(&guard, original.clone(), &rows, &mut data, None)
        .await
        .expect("compact");
    assert_eq!(returned, original);
    assert_eq!(requests.count(), 0);
    assert!(records(&data).is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn no_candidates_log_at_debug_without_calling_the_service() {
    let captured = Captured::default();
    let _subscriber = set_default(log_subscriber(tracing::Level::DEBUG, captured.clone()));
    let (base_url, requests, mut server) = serve(StatusCode::OK, reply(&[("e0", 0.0)])).await;
    let guard = guard_at(&base_url, args(&base_url));
    let original = inputs(no_candidates());
    let mut data = request_data();
    let returned = apply_request(&guard, original.clone(), &mut data)
        .await
        .expect("apply_guardrail");
    assert_eq!(returned, original);
    assert_eq!(requests.count(), 0);
    assert!(logs(&captured).contains(NO_CANDIDATES_LOG));
    server.shutdown().await;
}

#[tokio::test]
async fn all_relevant_answers_log_at_debug_and_leave_every_message() {
    let captured = Captured::default();
    let _subscriber = set_default(log_subscriber(tracing::Level::DEBUG, captured.clone()));
    let (base_url, requests, mut server) =
        serve(StatusCode::OK, reply(&[("e0", 1.0), ("e1", 1.0)])).await;
    let guard = guard_at(&base_url, args(&base_url));
    let original = inputs(two_exchanges());
    let mut data = request_data();
    let returned = apply_request(&guard, original.clone(), &mut data)
        .await
        .expect("apply_guardrail");
    assert_eq!(returned, original);
    assert_eq!(requests.count(), 1);
    assert!(logs(&captured).contains(ALL_RELEVANT_LOG));
    server.shutdown().await;
}

#[tokio::test]
async fn a_compaction_logs_the_exchange_and_character_counts() {
    let captured = Captured::default();
    let _subscriber = set_default(log_subscriber(tracing::Level::INFO, captured.clone()));
    let (base_url, _requests, mut server) =
        serve(StatusCode::OK, reply(&[("e0", 0.0), ("e1", 0.9)])).await;
    let guard = guard_at(&base_url, args(&base_url));
    let mut data = request_data();
    let returned = apply_request(&guard, inputs(two_exchanges()), &mut data)
        .await
        .expect("compaction");
    assert_eq!(
        returned.structured_messages,
        Some(dropped_once(&two_exchanges(), 2))
    );
    let removed = first_result_removed();
    assert!(logs(&captured).contains(&format!(
        "TypeSafe: evaluated 2 tool exchange(s), dropped 1, ~{removed} chars removed"
    )));
    server.shutdown().await;
}
