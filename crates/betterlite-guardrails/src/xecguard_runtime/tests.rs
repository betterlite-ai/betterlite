#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::{self, Write};
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::body::Bytes;
use axum::http::{StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::xecguard_runtime::Settings as RuntimeSettings;
use betterlite_http::proxy::base_builder;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::wire::ResponseBody;
use crate::third_party_init_b::xecguard::XecGuardArgs;
use crate::xecguard_primitives::DEFAULT_POLICIES;

use super::XecGuardGuard;

const GUARDRAIL_NAME: &str = "xecguard-guard";
const SAFE: &str = r#"{"decision":"SAFE","trace_id":"trace-1"}"#;
const UNSAFE: &str = r#"{"decision":"UNSAFE","trace_id":"trace-2","xecguard_result":[{"violated_policy_name":"HarmfulContent","rationale":"forbidden"}]}"#;
const GROUNDING_UNSAFE: &str = r#"{"decision":"UNSAFE","trace_id":"trace-3","xecguard_result":{"violated_rules_list":["RuleA"],"rationale":"not grounded"}}"#;
const USER_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#;
const RESPONSE_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"question"}],"response":{"choices":[{"message":{"role":"assistant","content":"answer"}}]}}"#;
const GROUNDING_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"question"}],"response":{"choices":[{"message":{"role":"assistant","content":"answer"}}]},"metadata":{"xecguard_grounding_documents":[{"document_id":"doc-1","context":"the context"}]}}"#;
const LOGGED_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}],"standard_logging_object":{"id":"call-1"}}"#;
const LOGGED_WITH_INFORMATION: &str = r#"{"litellm_params":{"metadata":{"standard_logging_guardrail_information":[{"guardrail_name":"older"}]}},"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{}}"#;
const LOGGED_WITH_EMPTY_INFORMATION: &str = r#"{"litellm_params":{"metadata":{"standard_logging_guardrail_information":[]}},"messages":[{"role":"user","content":"hello"}],"standard_logging_object":{}}"#;

static CLOCK_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
enum Case {
    ConstructorPrecedence,
    EnvironmentFallbacks,
    UnsetDefaults,
    MissingKey,
    DefaultPolicyList,
    ConfiguredPolicyList,
    SafeRequestScan,
    UnsafeRequestScan,
    SafeResponseScanWithoutGrounding,
    UnsafeResponseScan,
    GroundingBlock,
    BlockOnErrorTrue,
    BlockOnErrorFalse,
    LoggingSkipsExistingInformation,
    LoggingScansEmptyInformation,
    LoggingAppendsResponseEntry,
    LoggingAppendsRequestEntry,
    LoggingEmptyHistory,
    LoggingFailedScan,
    LoggingSwallowedFailure,
}

#[rstest]
#[case::constructor_precedence(Case::ConstructorPrecedence)]
#[case::environment_fallbacks(Case::EnvironmentFallbacks)]
#[case::unset_defaults(Case::UnsetDefaults)]
#[case::missing_key(Case::MissingKey)]
#[case::default_policy_list(Case::DefaultPolicyList)]
#[case::configured_policy_list(Case::ConfiguredPolicyList)]
#[case::safe_request_scan(Case::SafeRequestScan)]
#[case::unsafe_request_scan(Case::UnsafeRequestScan)]
#[case::safe_response_scan_without_grounding(Case::SafeResponseScanWithoutGrounding)]
#[case::unsafe_response_scan(Case::UnsafeResponseScan)]
#[case::grounding_block(Case::GroundingBlock)]
#[case::block_on_error_true(Case::BlockOnErrorTrue)]
#[case::block_on_error_false(Case::BlockOnErrorFalse)]
#[case::logging_skips_existing_information(Case::LoggingSkipsExistingInformation)]
#[case::logging_scans_empty_information(Case::LoggingScansEmptyInformation)]
#[case::logging_appends_response_entry(Case::LoggingAppendsResponseEntry)]
#[case::logging_appends_request_entry(Case::LoggingAppendsRequestEntry)]
#[case::logging_empty_history(Case::LoggingEmptyHistory)]
#[case::logging_failed_scan(Case::LoggingFailedScan)]
#[case::logging_swallowed_failure(Case::LoggingSwallowedFailure)]
#[tokio::test]
async fn xecguard_runtime_cases(#[case] case: Case) {
    match case {
        Case::ConstructorPrecedence => constructor_precedence(),
        Case::EnvironmentFallbacks => environment_fallbacks(),
        Case::UnsetDefaults => unset_defaults(),
        Case::MissingKey => missing_key(),
        Case::DefaultPolicyList => default_policy_list().await,
        Case::ConfiguredPolicyList => configured_policy_list().await,
        Case::SafeRequestScan => safe_request_scan().await,
        Case::UnsafeRequestScan => unsafe_request_scan().await,
        Case::SafeResponseScanWithoutGrounding => safe_response_scan().await,
        Case::UnsafeResponseScan => unsafe_response_scan().await,
        Case::GroundingBlock => grounding_block().await,
        Case::BlockOnErrorTrue => block_on_error_true().await,
        Case::BlockOnErrorFalse => block_on_error_false().await,
        Case::LoggingSkipsExistingInformation => logging_skips_existing().await,
        Case::LoggingScansEmptyInformation => logging_scans_empty_information().await,
        Case::LoggingAppendsResponseEntry => logging_appends_response_entry().await,
        Case::LoggingAppendsRequestEntry => logging_appends_request_entry().await,
        Case::LoggingEmptyHistory => logging_empty_history().await,
        Case::LoggingFailedScan => logging_failed_scan().await,
        Case::LoggingSwallowedFailure => logging_swallowed_failure().await,
    }
}

#[rstest]
#[case::true_text("true", true)]
#[case::one_text("1", true)]
#[case::yes_text("yes", true)]
#[case::upper_case("TRUE", true)]
#[case::mixed_case("Yes", true)]
#[case::false_text("false", false)]
#[case::zero_text("0", false)]
#[case::on_text("on", false)]
#[case::other_text("anything", false)]
fn xecguard_runtime_block_on_error_text(#[case] text: &str, #[case] expected: bool) {
    let mut environment = environment();
    environment.block_on_error = Some(text.to_owned());
    let guard = new_guard(args(), &environment, base(None, None));
    assert_eq!(guard.settings.block_on_error, expected);
}

#[test]
fn xecguard_runtime_sync_logging_hook_appends_an_entry() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let kwargs = logged(LOGGED_REQUEST);
    let (kwargs, _result) = guard.logging_hook(kwargs, chat_body("answer"));
    let entries = logged_entries(&kwargs);
    assert_eq!(entries.len(), 1);
    assert_eq!(text(field(&entries[0], "guardrail_status")), "success");
    assert_eq!(text(field(&entries[0], "guardrail_mode")), "logging_only");
    assert_eq!(seen(&mock).len(), 1);
}

fn constructor_precedence() {
    let mut input = args();
    input.api_base = Some("http://args.test/".to_owned());
    input.xecguard_model = Some("model-from-args".to_owned());
    input.policy_names = Some(vec!["Custom-Policy".to_owned()]);
    input.block_on_error = Some(true);
    input.grounding_strictness = Some("STRICT".to_owned());
    let guard = new_guard(input, &environment(), base(Some(GUARDRAIL_NAME), Some(2.5)));
    assert_eq!(guard.settings.api_key.expose_secret(), "key-from-args");
    assert_eq!(guard.settings.api_base, "http://args.test");
    assert_eq!(guard.settings.model, "model-from-args");
    assert_eq!(
        guard.settings.policy_names,
        vec!["Custom-Policy".to_owned()]
    );
    assert!(guard.settings.block_on_error);
    assert_eq!(guard.settings.grounding_strictness, "STRICT");
    assert_eq!(guard.settings.timeout, Duration::from_millis(2500));
    assert_eq!(
        guard.base.settings.guardrail_name.as_deref(),
        Some(GUARDRAIL_NAME)
    );
}

fn environment_fallbacks() {
    let mut input = args();
    input.api_key = None;
    input.api_base = None;
    input.policy_names = Some(Vec::new());
    let guard = new_guard(input, &environment(), base(None, None));
    assert_eq!(
        guard.settings.api_key.expose_secret(),
        "key-from-environment"
    );
    assert_eq!(guard.settings.api_base, "http://from-environment.test");
    assert_eq!(guard.settings.model, "xecguard_v2");
    assert!(!guard.settings.block_on_error);
    assert_eq!(guard.settings.grounding_strictness, "BALANCED");
    assert_eq!(guard.settings.timeout, Duration::from_secs(10));
    assert_eq!(guard.settings.policy_names, default_policies());
}

fn unset_defaults() {
    let guard = new_guard(args(), &RuntimeSettings::default(), base(None, None));
    assert_eq!(guard.settings.api_base, "https://api-xecguard.cycraft.ai");
    assert_eq!(guard.settings.model, "xecguard_v2");
    assert!(guard.settings.block_on_error);
    assert_eq!(guard.settings.grounding_strictness, "BALANCED");
    assert_eq!(guard.settings.timeout, Duration::from_secs(10));
    assert_eq!(guard.settings.policy_names, default_policies());
}

fn missing_key() {
    let mut input = args();
    input.api_key = None;
    let error = XecGuardGuard::new(
        &input,
        &RuntimeSettings::default(),
        base(None, None),
        client(),
    )
    .expect_err("a key is required");
    assert_eq!(
        error.to_string(),
        "XecGuard API key is required. Set XECGUARD_API_KEY in the environment or pass api_key in the guardrail config."
    );
}

async fn default_policy_list() {
    let mock = mock((200, SAFE), (200, SAFE));
    let mut input = args();
    input.api_base = Some(mock.base.clone());
    input.policy_names = Some(Vec::new());
    let guard = new_guard(input, &RuntimeSettings::default(), base(None, None));
    let mut data = request_data(USER_REQUEST);
    let returned = guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a pass");
    assert_eq!(returned, inputs(&["hello"]));
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/xecguard/v1/scan");
    assert_eq!(
        texts(field(&seen[0].body, "policy_names")),
        default_policies()
    );
    assert_eq!(text(field(&seen[0].body, "scan_type")), "input");
}

async fn configured_policy_list() {
    let mock = mock((200, SAFE), (200, SAFE));
    let mut input = args();
    input.api_base = Some(mock.base.clone());
    input.policy_names = Some(vec!["Custom-Policy".to_owned()]);
    let guard = new_guard(input, &RuntimeSettings::default(), base(None, None));
    let mut data = request_data(USER_REQUEST);
    guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a pass");
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(
        texts(field(&seen[0].body, "policy_names")),
        vec!["Custom-Policy".to_owned()]
    );
}

async fn safe_request_scan() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(USER_REQUEST);
    let original = inputs(&["hello"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a pass");
    assert_eq!(returned, original);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    let messages = messages_of(&seen[0].body);
    assert_eq!(messages.len(), 1);
    assert_eq!(text(field(&messages[0], "role")), "user");
    assert_eq!(text(field(&messages[0], "content")), "hello");
}

async fn unsafe_request_scan() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(USER_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("an UNSAFE scan blocks the request");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error").map_or("", text),
        "Blocked by XecGuard: policies=[HarmfulContent] trace_id=trace-2 rationale=forbidden"
    );
    assert_eq!(
        detail.get("guardrail_name").map_or("", text),
        GUARDRAIL_NAME
    );
    let response = detail.get("xecguard_response").expect("the scan result");
    assert_eq!(text(field(response, "decision")), "UNSAFE");
    assert_eq!(text(field(response, "trace_id")), "trace-2");
}

async fn safe_response_scan() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(RESPONSE_REQUEST);
    let original = inputs(&["answer"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("a pass");
    assert_eq!(returned, original);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/xecguard/v1/scan");
    assert_eq!(text(field(&seen[0].body, "scan_type")), "response");
    let messages = messages_of(&seen[0].body);
    assert_eq!(messages.len(), 2);
    assert_eq!(text(field(&messages[1], "role")), "assistant");
    assert_eq!(text(field(&messages[1], "content")), "answer");
}

async fn unsafe_response_scan() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(GROUNDING_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["answer"]),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect_err("an UNSAFE response scan blocks the response");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error").map_or("", text),
        "Blocked by XecGuard: policies=[HarmfulContent] trace_id=trace-2 rationale=forbidden"
    );
    assert_eq!(seen(&mock).len(), 1);
}

async fn grounding_block() {
    let mock = mock((200, SAFE), (200, GROUNDING_UNSAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(GROUNDING_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["answer"]),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect_err("an UNSAFE grounding check blocks the response");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error").map_or("", text),
        "Blocked by XecGuard grounding: rules=[RuleA] trace_id=trace-3 rationale=not grounded"
    );
    assert_eq!(
        detail.get("guardrail_name").map_or("", text),
        GUARDRAIL_NAME
    );
    let seen = seen(&mock);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].path, "/xecguard/v1/grounding");
    assert_eq!(text(field(&seen[1].body, "prompt")), "question");
    assert_eq!(text(field(&seen[1].body, "response")), "answer");
}

async fn block_on_error_true() {
    let mock = mock((500, "{}"), (200, SAFE));
    let guard = guard_with_block(&mock.base, true);
    let mut data = request_data(USER_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("block_on_error blocks an unreachable API");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    let message = detail.get("error").map_or("", text);
    assert!(
        message.starts_with("XecGuard API unreachable (block_on_error=True): "),
        "{message}"
    );
    assert!(message.contains("500"), "{message}");
    assert_eq!(
        detail.get("guardrail_name").map_or("", text),
        GUARDRAIL_NAME
    );
    assert!(detail.get("xecguard_response").is_none());
}

async fn block_on_error_false() {
    let mock = mock((500, "{}"), (200, SAFE));
    let guard = guard_with_block(&mock.base, false);
    let mut data = request_data(USER_REQUEST);
    let original = inputs(&["hello"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a fail-open pass");
    assert_eq!(returned, original);
    assert_eq!(seen(&mock).len(), 1);
}

async fn logging_skips_existing() {
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

async fn logging_scans_empty_information() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_WITH_EMPTY_INFORMATION);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert_eq!(logged_entries(&kwargs).len(), 1);
    assert_eq!(seen(&mock).len(), 1);
}

async fn logging_appends_response_entry() {
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

async fn logging_appends_request_entry() {
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

async fn logging_empty_history() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(r#"{"standard_logging_object":{}}"#);
    let before = values(&kwargs);
    guard.async_logging_hook(&mut kwargs, &empty_body()).await;
    assert_eq!(values(&kwargs), before);
    assert!(seen(&mock).is_empty());
}

async fn logging_failed_scan() {
    let mock = mock((500, "{}"), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut kwargs = logged(LOGGED_REQUEST);
    guard
        .async_logging_hook(&mut kwargs, &chat_body("answer"))
        .await;
    assert!(logged_entries(&kwargs).is_empty());
    assert_eq!(seen(&mock).len(), 1);
}

async fn logging_swallowed_failure() {
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

type Reply = (u16, &'static str);

#[derive(Clone)]
struct Seen {
    path: String,
    body: GuardrailValue,
}

struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinHandle<()>,
}

fn mock(scan: Reply, grounding: Reply) -> Mock {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("the listener");
    let base = format!("http://{}", listener.local_addr().expect("the address"));
    listener.set_nonblocking(true).expect("nonblocking");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let server = thread::Builder::new()
        .spawn(move || serve(listener, sink, scan, grounding))
        .expect("the mock server thread");
    Mock {
        base,
        seen,
        _server: server,
    }
}

fn reply_for(path: &str, scan: Reply, grounding: Reply) -> Reply {
    match path {
        "/xecguard/v1/scan" => scan,
        "/xecguard/v1/grounding" => grounding,
        _other => (404, "{}"),
    }
}

fn serve(listener: StdTcpListener, seen: Arc<Mutex<Vec<Seen>>>, scan: Reply, grounding: Reply) {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the mock runtime");
    runtime.block_on(async move {
        let listener = TcpListener::from_std(listener).expect("the tokio listener");
        let handler = move |uri: Uri, bytes: Bytes| {
            let sink = Arc::clone(&seen);
            let reply = reply_for(uri.path(), scan, grounding);
            async move {
                let body = serde_json::from_slice::<GuardrailValue>(&bytes)
                    .unwrap_or(GuardrailValue::Null);
                sink.lock().expect("the captured requests").push(Seen {
                    path: uri.path().to_owned(),
                    body,
                });
                let status = StatusCode::from_u16(reply.0).expect("the reply status");
                (status, [("content-type", "application/json")], reply.1)
            }
        };
        axum::serve(listener, Router::new().fallback(handler))
            .into_future()
            .await
            .expect("the mock server");
    });
}

fn seen(mock: &Mock) -> Vec<Seen> {
    mock.seen.lock().expect("the captured requests").clone()
}

fn args() -> XecGuardArgs {
    XecGuardArgs {
        name: GUARDRAIL_NAME.to_owned(),
        api_base: None,
        api_key: Some(SecretString::from("key-from-args")),
        xecguard_model: None,
        policy_names: None,
        block_on_error: None,
        grounding_strictness: None,
        timeout: None,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    }
}

fn environment() -> RuntimeSettings {
    RuntimeSettings {
        api_key: Some(SecretString::from("key-from-environment")),
        api_base: Some("http://from-environment.test".to_owned()),
        block_on_error: Some("false".to_owned()),
    }
}

fn base(name: Option<&str>, timeout: Option<f64>) -> GuardrailBase {
    GuardrailBase {
        settings: settings(name, timeout),
        mode: None,
    }
}

fn settings(name: Option<&str>, timeout: Option<f64>) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: name.map(str::to_owned),
        supported_event_hooks: None,
        event_hook: None,
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
        switches: GuardrailSwitches {
            use_native_during_call_hook: false,
            use_native_lifecycle_hooks: false,
            records_own_information: false,
            uses_apply_guardrail_interface: true,
            deny_text_on_error: false,
        },
    }
}

fn client() -> reqwest::Client {
    base_builder().build().expect("the client")
}

fn new_guard(args: XecGuardArgs, settings: &RuntimeSettings, base: GuardrailBase) -> XecGuardGuard {
    XecGuardGuard::new(&args, settings, base, client()).expect("the guard")
}

fn guard_at(api_base: &str) -> XecGuardGuard {
    let mut input = args();
    input.api_base = Some(api_base.to_owned());
    new_guard(
        input,
        &RuntimeSettings::default(),
        base(Some(GUARDRAIL_NAME), None),
    )
}

fn guard_with_block(api_base: &str, block_on_error: bool) -> XecGuardGuard {
    let mut input = args();
    input.api_base = Some(api_base.to_owned());
    input.block_on_error = Some(block_on_error);
    new_guard(
        input,
        &RuntimeSettings::default(),
        base(Some(GUARDRAIL_NAME), None),
    )
}

fn default_policies() -> Vec<String> {
    DEFAULT_POLICIES
        .iter()
        .map(|policy| (*policy).to_owned())
        .collect()
}

fn inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn request_data(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).expect("the request data")
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
    let logged = values(kwargs);
    let standard = field(&logged, "standard_logging_object");
    match field(standard, "guardrail_information") {
        GuardrailValue::Array(entries) => entries.clone(),
        _ => Vec::new(),
    }
}

fn blocked_object(failure: &GuardrailFailure) -> (u16, &IndexMap<String, GuardrailValue>) {
    match failure {
        GuardrailFailure::HttpObject { status, detail } => (*status, detail),
        other => panic!("an object failure: {other:?}"),
    }
}

fn messages_of(body: &GuardrailValue) -> Vec<GuardrailValue> {
    match field(body, "messages") {
        GuardrailValue::Array(messages) => messages.clone(),
        _ => Vec::new(),
    }
}

fn field<'value>(value: &'value GuardrailValue, name: &str) -> &'value GuardrailValue {
    match value {
        GuardrailValue::Object(fields) => fields.get(name).unwrap_or(&GuardrailValue::Null),
        _ => &GuardrailValue::Null,
    }
}

fn text(value: &GuardrailValue) -> &str {
    match value {
        GuardrailValue::Text(text) => text,
        _ => "",
    }
}

fn texts(value: &GuardrailValue) -> Vec<String> {
    match value {
        GuardrailValue::Array(items) => items.iter().filter_map(text_of).collect(),
        _ => Vec::new(),
    }
}

fn text_of(value: &GuardrailValue) -> Option<String> {
    match value {
        GuardrailValue::Text(text) => Some(text.clone()),
        _ => None,
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
