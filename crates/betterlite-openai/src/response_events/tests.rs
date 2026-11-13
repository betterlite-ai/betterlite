#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use tokio::runtime::Builder;
use tracing::subscriber::with_default;

use super::created_at::convert_created_at;
use super::event_decode::{
    DecodedStreamResponse, EventClass, decode_response_event, parse_terminal_event,
    parse_terminal_response, transform_streaming_response,
};
use super::{
    ApiKeySources, NativeStreamingSupport, RawResponse, ReplyBody, ResponsesProvider,
    ResponsesReply, SUPPORTS_NATIVE_WEBSOCKET, WorkloadIdentity, get_complete_url,
    should_fake_stream, transform_response, validate_environment,
};
use crate::api::responses_event_records::records::ResponsesTerminalEvent;
use crate::error::OpenAiError;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: i64 = 1_700_000_000;

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

fn fixture(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:05:00+00:00").unwrap(),
    };
    (state, env)
}

fn transform_text(
    text: &str,
    status: u16,
    now: i64,
    redactor: &Redactor,
) -> Result<ResponsesReply, OpenAiError> {
    let headers = IndexMap::new();
    let (mut state, env) = fixture(redactor);
    transform_response(
        &RawResponse {
            status,
            headers: &headers,
            text,
        },
        now,
        &mut state,
        &env,
    )
}

fn decision(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn chunk(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = decision(text) else {
        panic!("the test chunk must be an object");
    };
    fields
}

fn decode_bytes(text: &str) -> DecodedStreamResponse {
    decode_response_event(text.as_bytes()).unwrap()
}

fn decoded_value(decoded: &DecodedStreamResponse) -> Value {
    match decoded {
        DecodedStreamResponse::Known { record, .. } => serde_json::to_value(record).unwrap(),
        DecodedStreamResponse::Generic(event) => serde_json::to_value(event).unwrap(),
        DecodedStreamResponse::Lax { fields, .. } => {
            serde_json::to_value(Json::Object(fields.clone())).unwrap()
        }
    }
}

fn terminal_class(event: &ResponsesTerminalEvent) -> &'static str {
    match event {
        ResponsesTerminalEvent::Completed(_) => "ResponseCompletedEvent",
        ResponsesTerminalEvent::Incomplete(_) => "ResponseIncompleteEvent",
        ResponsesTerminalEvent::Failed(_) => "ResponseFailedEvent",
    }
}

fn capture<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, run);
    (result, fs::read_to_string(log.path()).unwrap())
}

#[derive(Clone, Copy)]
enum Scenario {
    CreatedAtConversion,
    LaxResponseValidation,
    MissingErrorCode,
    UnknownGenericEvent,
    CompletedIncompleteFailed,
    LastTerminalWins,
    NoTerminalEvent,
    NonNativeStreamingModel,
    KnownEventVariants,
    ResponsesAuthHeaders,
    ResponsesBaseUrl,
}

#[rstest]
#[case::created_at_conversion(Scenario::CreatedAtConversion)]
#[case::lax_response_validation(Scenario::LaxResponseValidation)]
#[case::missing_error_code(Scenario::MissingErrorCode)]
#[case::unknown_generic_event(Scenario::UnknownGenericEvent)]
#[case::completed_incomplete_failed(Scenario::CompletedIncompleteFailed)]
#[case::last_terminal_wins(Scenario::LastTerminalWins)]
#[case::no_terminal_event(Scenario::NoTerminalEvent)]
#[case::non_native_streaming_model(Scenario::NonNativeStreamingModel)]
#[case::known_event_variants(Scenario::KnownEventVariants)]
#[case::responses_auth_headers(Scenario::ResponsesAuthHeaders)]
#[case::responses_base_url(Scenario::ResponsesBaseUrl)]
fn response_events_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::CreatedAtConversion => created_at_conversion(),
        Scenario::LaxResponseValidation => lax_response_validation(),
        Scenario::MissingErrorCode => missing_error_code(),
        Scenario::UnknownGenericEvent => unknown_generic_event(),
        Scenario::CompletedIncompleteFailed => completed_incomplete_failed(),
        Scenario::LastTerminalWins => last_terminal_wins(),
        Scenario::NoTerminalEvent => no_terminal_event(),
        Scenario::NonNativeStreamingModel => non_native_streaming_model(),
        Scenario::KnownEventVariants => known_event_variants(),
        Scenario::ResponsesAuthHeaders => responses_auth_headers(),
        Scenario::ResponsesBaseUrl => responses_base_url(),
    }
}

fn created_at_conversion() {
    assert_eq!(convert_created_at(&decision("null"), NOW), Ok(NOW));
    assert_eq!(convert_created_at(&decision("12.9"), NOW), Ok(12));
    assert_eq!(convert_created_at(&decision("-12.9"), NOW), Ok(-12));
    assert_eq!(convert_created_at(&decision("7"), NOW), Ok(7));
    assert_eq!(convert_created_at(&decision("\"12.9\""), NOW), Ok(12));
    assert_eq!(convert_created_at(&decision("\" -12.9 \""), NOW), Ok(-12));
    assert_eq!(convert_created_at(&decision("\"abc\""), NOW), Ok(NOW));
    assert_eq!(convert_created_at(&decision("\"\""), NOW), Ok(NOW));
    assert_eq!(convert_created_at(&decision("\"nan\""), NOW), Ok(NOW));
    assert_eq!(convert_created_at(&decision("true"), NOW), Ok(NOW));
    convert_created_at(&decision("\"inf\""), NOW).unwrap_err();
    let redactor = redactor();
    for body in [r#"{"id":"r","output":[]}"#, "{not json", "[]", "null"] {
        let error = expect_error(transform_text(body, 400, NOW, &redactor));
        assert_eq!(status_of(&error), 400);
        assert_eq!(error.to_string(), body);
    }
}

fn expect_error(result: Result<ResponsesReply, OpenAiError>) -> OpenAiError {
    match result {
        Ok(_reply) => panic!("expected the reply to fail"),
        Err(error) => error,
    }
}

fn status_of(error: &OpenAiError) -> u16 {
    if let OpenAiError::Provider(provider) = error {
        provider.status_code
    } else {
        0
    }
}

fn reply_summary(reply: &ResponsesReply) -> Value {
    let mut summary = Map::new();
    match &reply.response {
        ReplyBody::Typed(response) => {
            summary.insert(String::from("id"), Value::String(response.id.clone()));
            summary.insert(String::from("created_at"), Value::from(response.created_at));
        }
        ReplyBody::Lax(fields) => {
            let id = fields
                .get("id")
                .map_or(Value::Null, |given| serde_json::to_value(given).unwrap());
            summary.insert(String::from("id"), id);
            let created_at = match fields.get("created_at") {
                Some(Json::Number(number)) => number.as_i64().unwrap(),
                _ => 0,
            };
            summary.insert(String::from("created_at"), Value::from(created_at));
        }
    }
    Value::Object(summary)
}

fn lax_response_validation() {
    let redactor = redactor();
    let headers = IndexMap::from([
        (String::from("x-request-id"), String::from("req-1")),
        (
            String::from("x-ratelimit-remaining-requests"),
            String::from("9"),
        ),
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
    ]);
    let body = r#"{"id":7,"created_at":5,"output":"not-a-list"}"#;
    let (mut state, env) = fixture(&redactor);
    let reply = transform_response(
        &RawResponse {
            status: 200,
            headers: &headers,
            text: body,
        },
        9,
        &mut state,
        &env,
    )
    .unwrap();
    assert!(matches!(reply.response, ReplyBody::Lax(_)));
    assert_eq!(reply_summary(&reply), value(r#"{"id":7,"created_at":5}"#));
    assert_eq!(reply.headers, headers);
    recorded_headers(&reply);
    recorded_post_call(&state, body);
    let typed = transform_text(
        r#"{"id":"r","created_at":5,"output":[]}"#,
        200,
        9,
        &redactor,
    );
    assert!(matches!(typed.unwrap().response, ReplyBody::Typed(_)));
    let (_result, logs) = capture(|| transform_text(body, 200, 9, &redactor));
    assert!(logs.contains("did not validate"), "{logs}");
}

fn recorded_headers(reply: &ResponsesReply) {
    assert_eq!(
        reply.additional_headers.get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("req-1")))
    );
    assert_eq!(
        reply
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(
        reply
            .additional_headers
            .get("llm_provider-x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
}

fn recorded_post_call(state: &LoggingState, body: &str) {
    let details = &state.model_call_details;
    assert_eq!(
        details.get("original_response"),
        Some(&Json::Text(String::from(body)))
    );
    let recorded = serde_json::to_value(details.get("additional_args").unwrap()).unwrap();
    assert_eq!(recorded, value(r#"{"complete_input_dict":{}}"#));
}

fn missing_error_code() {
    let missing = decode_bytes(
        r#"{"type":"error","sequence_number":1,"error":{"type":"invalid_request_error","message":"bad"}}"#,
    );
    assert_eq!(missing.class().class_name(), "ErrorEvent");
    assert_eq!(
        decoded_value(&missing).pointer("/error/code"),
        Some(&value("\"unknown_error\""))
    );
    let nulled = decode_bytes(
        r#"{"type":"error","sequence_number":1,"error":{"type":"invalid_request_error","code":null,"message":"bad"}}"#,
    );
    assert_eq!(
        decoded_value(&nulled).pointer("/error/code"),
        Some(&value("\"unknown_error\""))
    );
    let kept = decode_bytes(
        r#"{"type":"error","sequence_number":1,"error":{"type":"invalid_request_error","code":"bad_request","message":"bad"}}"#,
    );
    assert_eq!(
        decoded_value(&kept).pointer("/error/code"),
        Some(&value("\"bad_request\""))
    );
    let nondict = decode_bytes(r#"{"type":"error","sequence_number":1,"error":"oops"}"#);
    assert!(matches!(nondict, DecodedStreamResponse::Lax { .. }));
    assert_eq!(
        decoded_value(&nondict).get("error"),
        Some(&value("\"oops\""))
    );
    let original =
        chunk(r#"{"type":"response.created","response":{"id":"r","created_at":1,"output":[]}}"#);
    let untouched = original.clone();
    let _decoded = transform_streaming_response(&original);
    assert_eq!(original, untouched);
}

fn unknown_generic_event() {
    for event_type in [
        "vendor.future",
        "response.reasoning_summary_part.added",
        "response.reasoning_summary_text.delta",
        "response.reasoning_summary_text.done",
        "response.reasoning_summary_part.done",
        "response.custom_tool_call_input.delta",
        "response.custom_tool_call_input.done",
        "response.shell_call.in_progress",
        "response.shell_call.completed",
        "response.shell_call_output.done",
    ] {
        let decoded = decode_bytes(&format!(
            r#"{{"type":"{event_type}","item_id":"i","payload":{{"n":1}}}}"#
        ));
        assert_eq!(decoded.class().class_name(), "GenericEvent", "{event_type}");
        assert_eq!(
            decoded_value(&decoded).get("payload"),
            Some(&value(r#"{"n":1}"#)),
            "{event_type}"
        );
    }
    let missing_type = decode_bytes(r#"{"delta":"x"}"#);
    assert!(matches!(missing_type.class(), EventClass::Generic));
    let number_type = decode_bytes(r#"{"type":5,"delta":"x"}"#);
    assert!(matches!(number_type.class(), EventClass::Generic));
    assert_eq!(decoded_value(&number_type).get("type"), Some(&value("5")));
}

fn completed_incomplete_failed() {
    for (event_type, class) in [
        ("response.completed", "ResponseCompletedEvent"),
        ("response.incomplete", "ResponseIncompleteEvent"),
        ("response.failed", "ResponseFailedEvent"),
    ] {
        let chunks = [format!(
            r#"data: {{"type":"{event_type}","response":{{"id":"resp-1","created_at":7,"output":[]}}}}"#
        )];
        let response = parse_terminal_response(&chunks).unwrap();
        assert_eq!(response.id, "resp-1");
        assert_eq!(response.created_at, 7);
        let event = parse_terminal_event(&chunks).unwrap();
        assert_eq!(terminal_class(&event), class);
    }
}

fn last_terminal_wins() {
    let chunks = [
        r#"data: {"type":"response.completed","response":{"id":"first","created_at":1,"output":[]}}"#.to_owned(),
        r#"data: {"type":"response.incomplete","response":{"id":"second","created_at":2,"output":[]}}"#.to_owned(),
        String::from("garbage"),
        r#"data:{"type":"response.failed","response":{"id":"third","created_at":3,"output":[]}}"#.to_owned(),
    ];
    let event = parse_terminal_event(&chunks).unwrap();
    assert_eq!(terminal_class(&event), "ResponseIncompleteEvent");
    let spaced = [
        r#"{"type":"response.completed","response":{"id":"bare","created_at":4,"output":[]}}"#.to_owned(),
        r#"data:  {"type":"response.failed","response":{"id":"spaced","created_at":5,"output":[]}}"#.to_owned(),
    ];
    let spaced_event = parse_terminal_event(&spaced).unwrap();
    assert_eq!(terminal_class(&spaced_event), "ResponseFailedEvent");
    assert_eq!(parse_terminal_response(&spaced).unwrap().id, "spaced");
}

fn no_terminal_event() {
    assert!(parse_terminal_event(&[]).is_none());
    let chunks = [
        r#"data: {"type":"response.created","response":{"id":"r","created_at":1,"output":[]}}"#.to_owned(),
        r#"data: {"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"d"}"#.to_owned(),
    ];
    assert!(parse_terminal_event(&chunks).is_none());
    assert!(parse_terminal_response(&chunks).is_none());
    let invalid = [
        r#"data: {"type":"response.completed","response":{"id":"r","created_at":1,"output":"bad"}}"#.to_owned(),
    ];
    assert!(parse_terminal_event(&invalid).is_none());
}

struct Capabilities {
    answer: Json,
}

impl NativeStreamingSupport for Capabilities {
    type Error = IoError;

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Result<Json, IoError>> + Send {
        ready(match &self.answer {
            Json::Text(text) if text == "error" => Err(IoError::other("the catalog lookup failed")),
            other @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_)) => Ok(other.clone()),
        })
    }
}

fn non_native_streaming_model() {
    let runtime = Builder::new_current_thread().build().unwrap();
    let mut probes = Vec::new();
    for (model, stream, answer) in [
        (Some("m"), None, Json::Bool(false)),
        (Some("m"), Some(false), Json::Bool(false)),
        (None, Some(true), Json::Bool(false)),
        (Some("m"), Some(true), Json::Bool(false)),
        (Some("m"), Some(true), Json::Bool(true)),
        (Some("m"), Some(true), Json::Text(String::from("sometimes"))),
        (Some("m"), Some(true), Json::Text(String::from("error"))),
    ] {
        let mut capabilities = Capabilities { answer };
        probes.push(runtime.block_on(should_fake_stream(
            model,
            stream,
            Some("openai"),
            &mut capabilities,
        )));
    }
    assert_eq!(probes, [false, false, false, true, false, false, false]);
    const { assert!(SUPPORTS_NATIVE_WEBSOCKET) };
}

fn known_event_variants() {
    known_lifecycle_events();
    known_content_events();
    known_search_and_mcp_events();
    known_invalid_payloads();
}

fn known_lifecycle_events() {
    for event_type in [
        "response.created",
        "response.in_progress",
        "response.completed",
        "response.failed",
        "response.incomplete",
    ] {
        let decoded = decode_bytes(&format!(
            r#"{{"type":"{event_type}","response":{{"id":"r","created_at":1,"output":[]}}}}"#
        ));
        assert!(
            matches!(decoded, DecodedStreamResponse::Known { .. }),
            "{event_type}"
        );
    }
}

fn known_content_events() {
    for event in [
        r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"message"}}"#,
        r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"message"}}"#,
        r#"{"type":"response.content_part.added","item_id":"i","output_index":0,"content_index":0,"part":{"type":"output_text","text":"t"}}"#,
        r#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":0,"part":{"type":"refusal","refusal":"no"}}"#,
        r#"{"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"d"}"#,
        r#"{"type":"response.output_text.annotation.added","item_id":"i","output_index":0,"content_index":0,"annotation_index":1,"annotation":{"type":"url_citation"}}"#,
        r#"{"type":"response.output_text.done","item_id":"i","output_index":0,"content_index":0,"text":"t"}"#,
        r#"{"type":"response.refusal.delta","item_id":"i","output_index":0,"content_index":0,"delta":"d"}"#,
        r#"{"type":"response.refusal.done","item_id":"i","output_index":0,"content_index":0,"refusal":"no"}"#,
        r#"{"type":"response.function_call_arguments.delta","item_id":"i","output_index":0,"delta":"{"}"#,
        r#"{"type":"response.function_call_arguments.done","item_id":"i","output_index":0,"arguments":"{}"}"#,
        r#"{"type":"image_generation.partial_image","partial_image_index":0,"b64_json":"YQ=="}"#,
        r#"{"type":"error","sequence_number":1,"error":{"type":"invalid_request_error","code":"bad","message":"m"}}"#,
    ] {
        let decoded = decode_bytes(event);
        assert!(
            matches!(decoded, DecodedStreamResponse::Known { .. }),
            "{event}"
        );
    }
}

fn known_search_and_mcp_events() {
    for event_type in [
        "response.file_search_call.in_progress",
        "response.file_search_call.searching",
        "response.file_search_call.completed",
        "response.web_search_call.in_progress",
        "response.web_search_call.searching",
        "response.web_search_call.completed",
        "response.mcp_list_tools.in_progress",
        "response.mcp_list_tools.completed",
        "response.mcp_list_tools.failed",
        "response.mcp_call.in_progress",
        "response.mcp_call.completed",
        "response.mcp_call.failed",
    ] {
        let decoded = decode_bytes(&format!(
            r#"{{"type":"{event_type}","sequence_number":1,"item_id":"i","output_index":0}}"#
        ));
        assert!(
            matches!(decoded, DecodedStreamResponse::Known { .. }),
            "{event_type}"
        );
    }
    for event_type in [
        "response.mcp_call_arguments.delta",
        "response.mcp_call_arguments.done",
    ] {
        let decoded = decode_bytes(&format!(
            r#"{{"type":"{event_type}","sequence_number":1,"item_id":"i","output_index":0,"delta":"{{"}}"#
        ));
        assert!(
            matches!(decoded, DecodedStreamResponse::Known { .. }),
            "{event_type}"
        );
    }
}

fn known_invalid_payloads() {
    let annotated = decode_bytes(
        r#"{"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"d","sequence_number":3}"#,
    );
    assert_eq!(
        decoded_value(&annotated).get("sequence_number"),
        Some(&value("3"))
    );
    let invalid = decode_bytes(
        r#"{"type":"response.output_text.delta","item_id":5,"output_index":0,"content_index":0,"delta":"d"}"#,
    );
    assert!(matches!(invalid, DecodedStreamResponse::Lax { .. }));
    assert_eq!(invalid.class().class_name(), "OutputTextDeltaEvent");
    assert_eq!(decoded_value(&invalid).get("item_id"), Some(&value("5")));
    let (_decoded, logs) = capture(|| {
        decode_bytes(
            r#"{"type":"response.output_text.delta","item_id":5,"output_index":0,"content_index":0,"delta":"d"}"#,
        )
    });
    assert!(logs.contains("did not validate"), "{logs}");
}

struct Identity {
    enabled: bool,
    calls: usize,
}

impl WorkloadIdentity for Identity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Option<SecretString> {
        self.calls = self.calls.saturating_add(1);
        if !self.enabled {
            return None;
        }
        Some(SecretString::from(format!(
            "wi:{}",
            api_base.unwrap_or("default")
        )))
    }
}

fn sources<'keys>(
    params: Option<&'keys SecretString>,
    litellm: Option<&'keys SecretString>,
    openai: Option<&'keys SecretString>,
    env: Option<&'keys SecretString>,
) -> ApiKeySources<'keys> {
    ApiKeySources {
        litellm_params: params,
        litellm,
        openai,
        environment: env,
    }
}

fn responses_auth_headers() {
    auth_key_precedence();
    auth_content_type();
    auth_openai_and_environment();
    auth_missing_key();
    auth_identity_branch();
}

fn auth_key_precedence() {
    let params_key = SecretString::from("params-key");
    let litellm_key = SecretString::from("litellm-key");
    let openai_key = SecretString::from("openai-key");
    let env_key = SecretString::from("env-key");
    let mut identity = Identity {
        enabled: false,
        calls: 0,
    };
    let defaulted = validate_environment(
        BTreeMap::new(),
        sources(
            Some(&params_key),
            Some(&litellm_key),
            Some(&openai_key),
            Some(&env_key),
        ),
        None,
        ResponsesProvider::OpenAi,
        &mut identity,
    );
    assert_eq!(
        defaulted.get("Authorization").map(String::as_str),
        Some("Bearer params-key")
    );
    assert_eq!(
        defaulted.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

fn auth_content_type() {
    let litellm_key = SecretString::from("litellm-key");
    let mut identity = Identity {
        enabled: false,
        calls: 0,
    };
    let lowercase = validate_environment(
        BTreeMap::from([(String::from("content-type"), String::from("text/plain"))]),
        sources(None, Some(&litellm_key), None, None),
        None,
        ResponsesProvider::OpenAi,
        &mut identity,
    );
    assert_eq!(
        lowercase.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        lowercase.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        lowercase.get("Authorization").map(String::as_str),
        Some("Bearer litellm-key")
    );
}

fn auth_openai_and_environment() {
    let openai_key = SecretString::from("openai-key");
    let env_key = SecretString::from("env-key");
    let mut identity = Identity {
        enabled: false,
        calls: 0,
    };
    let openai_only = validate_environment(
        BTreeMap::new(),
        sources(None, None, Some(&openai_key), Some(&env_key)),
        None,
        ResponsesProvider::OpenAi,
        &mut identity,
    );
    assert_eq!(
        openai_only.get("Authorization").map(String::as_str),
        Some("Bearer openai-key")
    );
    let environment_only = validate_environment(
        BTreeMap::new(),
        sources(None, None, None, Some(&env_key)),
        None,
        ResponsesProvider::OpenAi,
        &mut identity,
    );
    assert_eq!(
        environment_only.get("Authorization").map(String::as_str),
        Some("Bearer env-key")
    );
}

fn auth_missing_key() {
    let empty = SecretString::from("");
    let mut identity = Identity {
        enabled: false,
        calls: 0,
    };
    let missing = validate_environment(
        BTreeMap::from([(String::from("Authorization"), String::from("old"))]),
        sources(Some(&empty), None, None, Some(&empty)),
        None,
        ResponsesProvider::OpenAi,
        &mut identity,
    );
    assert_eq!(
        missing.get("Authorization").map(String::as_str),
        Some("Bearer None")
    );
}

fn auth_identity_branch() {
    let mut resolving = Identity {
        enabled: true,
        calls: 0,
    };
    let replaced = validate_environment(
        BTreeMap::new(),
        sources(None, None, None, None),
        Some("https://identity.example/v1"),
        ResponsesProvider::OpenAi,
        &mut resolving,
    );
    assert_eq!(
        replaced.get("Authorization").map(String::as_str),
        Some("Bearer wi:https://identity.example/v1")
    );
    assert_eq!(resolving.calls, 1);
    let litellm_key = SecretString::from("litellm-key");
    let mut untouched = Identity {
        enabled: true,
        calls: 0,
    };
    let other = validate_environment(
        BTreeMap::new(),
        sources(None, Some(&litellm_key), None, None),
        Some("https://identity.example/v1"),
        ResponsesProvider::Other,
        &mut untouched,
    );
    assert_eq!(
        other.get("Authorization").map(String::as_str),
        Some("Bearer litellm-key")
    );
    assert_eq!(untouched.calls, 0);
}

fn responses_base_url() {
    assert_eq!(
        get_complete_url(None, None, None, None),
        "https://api.openai.com/v1/responses"
    );
    assert_eq!(
        get_complete_url(Some("https://a.example/v1/"), None, None, None),
        "https://a.example/v1/responses"
    );
    assert_eq!(
        get_complete_url(Some(""), Some("https://b.example//"), None, None),
        "https://b.example/responses"
    );
    assert_eq!(
        get_complete_url(None, None, Some("https://c.example/"), None),
        "https://c.example/responses"
    );
    assert_eq!(
        get_complete_url(None, None, Some(""), Some("https://d.example/v1")),
        "https://d.example/v1/responses"
    );
    assert_eq!(
        get_complete_url(
            Some("https://e.example/v1/embeddings?x=1"),
            Some("https://ignored.example"),
            None,
            None
        ),
        "https://e.example/v1/embeddings?x=1/responses"
    );
    assert_eq!(
        get_complete_url(Some("https://f.example/v1"), None, None, None),
        "https://f.example/v1/responses"
    );
}

#[test]
fn openai_response_events_credential_identity_isolation() {
    credential_isolation_keys();
    credential_isolation_tokens();
}

fn credential_isolation_keys() {
    let first_key = SecretString::from("first-api-key");
    let second_key = SecretString::from("second-api-key");
    let mut none = Identity {
        enabled: false,
        calls: 0,
    };
    let first = validate_environment(
        BTreeMap::new(),
        sources(Some(&first_key), None, None, None),
        Some("https://first.example/v1"),
        ResponsesProvider::OpenAi,
        &mut none,
    );
    let second = validate_environment(
        BTreeMap::new(),
        sources(Some(&second_key), None, None, None),
        Some("https://second.example/v1"),
        ResponsesProvider::OpenAi,
        &mut none,
    );
    assert_eq!(
        first.get("Authorization").map(String::as_str),
        Some("Bearer first-api-key")
    );
    assert_eq!(
        second.get("Authorization").map(String::as_str),
        Some("Bearer second-api-key")
    );
    assert_ne!(first, second);
    assert!(
        !first
            .values()
            .any(|given| given.contains(second_key.expose_secret()))
    );
}

fn credential_isolation_tokens() {
    let mut first_identity = Identity {
        enabled: true,
        calls: 0,
    };
    let mut second_identity = Identity {
        enabled: true,
        calls: 0,
    };
    let first = validate_environment(
        BTreeMap::new(),
        sources(None, None, None, None),
        Some("https://first.example/v1"),
        ResponsesProvider::OpenAi,
        &mut first_identity,
    );
    let second = validate_environment(
        BTreeMap::new(),
        sources(None, None, None, None),
        Some("https://second.example/v1"),
        ResponsesProvider::OpenAi,
        &mut second_identity,
    );
    assert_eq!(
        first.get("Authorization").map(String::as_str),
        Some("Bearer wi:https://first.example/v1")
    );
    assert_eq!(
        second.get("Authorization").map(String::as_str),
        Some("Bearer wi:https://second.example/v1")
    );
    assert_ne!(first, second);
    assert_eq!((first_identity.calls, second_identity.calls), (1, 1));
}

#[test]
fn openai_response_events_secret_safe_failure() {
    let secret = String::from("fixture-secret-api-key");
    let key = SecretString::from(secret.clone());
    let api_sources = sources(Some(&key), None, None, None);
    let debugged = format!("{api_sources:?}");
    assert!(!debugged.contains(&secret), "{debugged}");
    let redactor = redactor();
    let error = expect_error(transform_text("{not json", 400, NOW, &redactor));
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(&secret), "{shown}");
    let mut identity = Identity {
        enabled: true,
        calls: 0,
    };
    let headers = validate_environment(
        BTreeMap::new(),
        sources(None, None, None, None),
        Some("https://secret.example/v1"),
        ResponsesProvider::OpenAi,
        &mut identity,
    );
    assert!(!format!("{headers:?}").contains(&secret));
    let mut failing = FailingCapabilities;
    let (fake, logs) = capture(|| {
        let runtime = Builder::new_current_thread().build().unwrap();
        runtime.block_on(should_fake_stream(
            Some("fixture-model"),
            Some(true),
            None,
            &mut failing,
        ))
    });
    assert!(!fake);
    assert!(logs.contains("native streaming lookup failed"), "{logs}");
    assert!(!logs.contains("fixture-secret-token"), "{logs}");
}

struct FailingCapabilities;

impl NativeStreamingSupport for FailingCapabilities {
    type Error = IoError;

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Result<Json, IoError>> + Send {
        ready(Err(IoError::other("fixture-secret-token")))
    }
}

#[rstest]
fn replays_the_openai_response_events_corpus(
    #[files("fuzz/corpus/openai_response_events/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let decoded = decode_response_event(&bytes);
    if name.starts_with("malformed") {
        assert!(decoded.is_err(), "{name} should be malformed");
    } else {
        assert!(decoded.is_ok(), "{name} should decode");
    }
}
