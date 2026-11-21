#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use betterlite_logging::guardrail_snapshot::request_guardrail_fingerprint;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::admin::{
    ALLOW_CLIENT_MESSAGE_REDACTION_OPT_OUT_KEY, ALLOW_CLIENT_MOCK_RESPONSE_KEY,
    ALLOW_CLIENT_PRICING_OVERRIDE_KEY, key_or_team_metadata_flag_is_true,
};
use super::identity::{
    anthropic_session_id_from_metadata, get_chain_id_from_headers, sanitize_for_log,
    stampable_key_hash,
};
use super::metadata::{
    MetadataField, is_llm_inference_route, metadata_variable_name_for_route,
    metadata_variable_name_from_data, promoted_trace_control_fields,
};
use super::session::apply_missing_session_id_policy;
use super::snapshot::refresh_proxy_server_request_body_snapshot;
use super::strip::strip_untrusted_request_header_controls;
use super::{
    MetadataPrincipal, MetadataSafetySettings, RequestMetadataSafetyInput,
    sanitize_request_metadata,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn body(entries: Vec<(&str, Json)>) -> RequestData {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn field<'data>(data: &'data RequestData, name: &str) -> &'data IndexMap<String, Json> {
    match data.get(name) {
        Some(Json::Object(values)) => values,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            panic!("{name} is not an object")
        }
    }
}

fn settings() -> MetadataSafetySettings {
    MetadataSafetySettings {
        turn_off_message_logging: true,
        ..MetadataSafetySettings::default()
    }
}

fn input(data: RequestData, route: &str) -> RequestMetadataSafetyInput {
    RequestMetadataSafetyInput {
        data,
        headers: IndexMap::new(),
        route: route.to_owned(),
        principal: MetadataPrincipal::default(),
    }
}

fn input_with_headers(
    data: RequestData,
    headers: &[(&str, &str)],
    route: &str,
) -> RequestMetadataSafetyInput {
    RequestMetadataSafetyInput {
        data,
        headers: headers
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        route: route.to_owned(),
        principal: MetadataPrincipal::default(),
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<(Level, String)>>>);

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if visitor.0.is_empty() {
            return;
        }
        let mut logs = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        logs.push((*event.metadata().level(), visitor.0));
    }
}

fn capture() -> (Capture, DefaultGuard) {
    let logs = Capture::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

fn messages(logs: &Capture) -> Vec<(Level, String)> {
    logs.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn forged_data() -> RequestData {
    let mut data = body(vec![
        ("model", text("gpt-4")),
        ("turn_off_message_logging", Json::Bool(false)),
        ("input_cost_per_token", text("0.001")),
        ("output_cost_per_token", text("0.002")),
        ("dd_api_key", text("client-dd")),
    ]);
    data.insert(
        "litellm_trusted_callback_vars".to_owned(),
        object(vec![("otel", text("client"))]),
    );
    data.insert("litellm_params".to_owned(), forged_litellm_params());
    data.insert("metadata".to_owned(), forged_metadata());
    data.insert(
        "litellm_metadata".to_owned(),
        object(vec![
            ("original_model_group", text("gpt-4")),
            ("turn_off_message_logging", text("off")),
            ("kept", text("value")),
        ]),
    );
    data
}

fn forged_litellm_params() -> Json {
    object(vec![(
        "metadata",
        object(vec![
            ("gcs_bucket_name", text("client-bucket")),
            ("dd_site", text("client-host")),
            ("kept", text("value")),
        ]),
    )])
}

fn forged_metadata() -> Json {
    object(vec![
        ("model_info", object(vec![])),
        ("standard_logging_guardrail_information", Json::List(vec![])),
        ("attempted_fallbacks", Json::List(vec![])),
        ("request_retry_count", Json::Number(Number::from(3))),
        ("turn_off_message_logging", Json::Bool(false)),
        (
            "headers",
            object(vec![
                ("litellm-disable-message-redaction", text("1")),
                ("x-keep", text("1")),
            ]),
        ),
        ("kept", text("value")),
    ])
}

#[test]
fn forged_control_stripped() {
    let (logs, _guard) = capture();
    let request = input_with_headers(
        forged_data(),
        &[("litellm-disable-message-redaction", "1")],
        "/v1/chat/completions",
    );
    let result = sanitize_request_metadata(&request, &settings()).unwrap();
    assert_pricing_stripped(&result.data);
    assert_callback_credentials_stripped(&result.data);
    assert_router_reserved_stripped(&result.data);
    assert_opt_out_stripped(&result.data, &result.headers);
    assert_eq!(
        field(&result.data, "metadata").get("kept"),
        Some(&text("value"))
    );
    assert_eq!(
        field(&result.data, "litellm_metadata").get("kept"),
        Some(&text("value"))
    );
    assert_strip_diagnostics(&messages(&logs));
}

#[test]
fn true_redaction_opt_outs_are_kept() {
    let data = body(vec![
        ("turn_off_message_logging", Json::Bool(true)),
        (
            "metadata",
            object(vec![("turn_off_message_logging", Json::Bool(true))]),
        ),
    ]);
    let result = sanitize_request_metadata(&input(data, "/v1/chat/completions"), &settings())
        .expect("the slots are objects");
    assert_eq!(
        result.data.get("turn_off_message_logging"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        field(&result.data, "metadata").get("turn_off_message_logging"),
        Some(&Json::Bool(true))
    );
}

#[test]
fn metadata_string_length_boundary() {
    let bounded = MetadataSafetySettings {
        limits: ApiLimits {
            max_json_bytes: 10,
            ..ApiLimits::default()
        },
        ..settings()
    };
    for (raw, accepted) in [("{\"a\":\"bc\"}", true), ("{\"a\":\"bcd\"}", false)] {
        let data = body(vec![("metadata", text(raw))]);
        let outcome = sanitize_request_metadata(&input(data, "/v1/chat/completions"), &bounded);
        assert_eq!(outcome.is_ok(), accepted);
    }
}

fn assert_pricing_stripped(data: &RequestData) {
    assert!(!data.contains_key("input_cost_per_token"));
    assert!(!data.contains_key("output_cost_per_token"));
    let metadata = field(data, "metadata");
    assert!(!metadata.contains_key("model_info"));
    assert!(!metadata.contains_key("standard_logging_guardrail_information"));
}

fn assert_callback_credentials_stripped(data: &RequestData) {
    assert!(!data.contains_key("dd_api_key"));
    assert!(!data.contains_key("litellm_trusted_callback_vars"));
    let nested = field(field(data, "litellm_params"), "metadata");
    assert!(!nested.contains_key("gcs_bucket_name"));
    assert!(!nested.contains_key("dd_site"));
    assert_eq!(nested.get("kept"), Some(&text("value")));
}

fn assert_router_reserved_stripped(data: &RequestData) {
    let metadata = field(data, "metadata");
    assert!(!metadata.contains_key("attempted_fallbacks"));
    assert!(!metadata.contains_key("request_retry_count"));
    assert!(!field(data, "litellm_metadata").contains_key("original_model_group"));
}

fn assert_opt_out_stripped(data: &RequestData, headers: &IndexMap<String, String>) {
    assert!(!data.contains_key("turn_off_message_logging"));
    let metadata = field(data, "metadata");
    assert!(!metadata.contains_key("turn_off_message_logging"));
    assert!(!field(data, "litellm_metadata").contains_key("turn_off_message_logging"));
    assert!(!field(metadata, "headers").contains_key("litellm-disable-message-redaction"));
    assert_eq!(field(metadata, "headers").get("x-keep"), Some(&text("1")));
    assert!(!headers.contains_key("litellm-disable-message-redaction"));
}

fn assert_strip_diagnostics(events: &[(Level, String)]) {
    assert!(!events.is_empty());
    assert!(
        events
            .iter()
            .all(|(level, _message)| *level == Level::DEBUG)
    );
    let joined = events
        .iter()
        .map(|(_level, message)| message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains(
        "pricing fields from request body: input_cost_per_token, output_cost_per_token, metadata.model_info, metadata.standard_logging_guardrail_information."
    ));
    assert!(joined.contains(
        "callback credentials from request: body.dd_api_key, litellm_params.metadata.dd_site, litellm_params.metadata.gcs_bucket_name."
    ));
    assert!(joined.contains(
        "router-reserved metadata field from request body: metadata.attempted_fallbacks"
    ));
    assert!(joined.contains(
        "router-reserved metadata field from request body: metadata.request_retry_count"
    ));
    assert!(joined.contains(
        "router-reserved metadata field from request body: litellm_metadata.original_model_group"
    ));
    assert!(joined.contains(
        "message-redaction opt-out fields from request body: turn_off_message_logging, litellm_metadata.turn_off_message_logging, metadata.turn_off_message_logging."
    ));
}

#[rstest]
#[case::exact("litellm-disable-message-redaction", false, false)]
#[case::mixed_case("Litellm-Disable-Message-Redaction", false, false)]
#[case::allowed_by_admin("litellm-disable-message-redaction", true, true)]
#[case::unrelated("x-keep", false, true)]
fn forged_header_control_variants(#[case] name: &str, #[case] allow: bool, #[case] kept: bool) {
    let mut headers: IndexMap<String, String> = IndexMap::from([(name.to_owned(), "1".to_owned())]);
    strip_untrusted_request_header_controls(&mut headers, allow);
    assert_eq!(headers.contains_key(name), kept);
}

#[derive(Clone, Copy, Debug)]
enum ParseOutcome {
    Object(&'static str),
    Rejected(&'static str),
}

#[rstest]
#[case::object_text(
    "metadata",
    "{\"session_id\":\"s-1\"}",
    ParseOutcome::Object("session_id")
)]
#[case::litellm_metadata_text("litellm_metadata", "{\"kept\":true}", ParseOutcome::Object("kept"))]
#[case::array_text("metadata", "[1,2]", ParseOutcome::Rejected("an array"))]
#[case::integer_text("litellm_metadata", "42", ParseOutcome::Rejected("an integer"))]
#[case::float_text("metadata", "1.5", ParseOutcome::Rejected("a number"))]
#[case::boolean_text("metadata", "true", ParseOutcome::Rejected("a boolean"))]
#[case::quoted_text("metadata", "\"text\"", ParseOutcome::Rejected("a string"))]
#[case::not_json("metadata", "not json", ParseOutcome::Rejected("a string"))]
fn metadata_string_parsed(#[case] slot: &str, #[case] raw: &str, #[case] outcome: ParseOutcome) {
    let data = body(vec![(slot, text(raw))]);
    let result = sanitize_request_metadata(&input(data, "/v1/chat/completions"), &settings());
    match outcome {
        ParseOutcome::Object(key) => {
            let accepted = result.unwrap();
            let parsed = field(&accepted.data, slot);
            assert!(parsed.contains_key(key));
            assert!(matches!(accepted.data.get(slot), Some(Json::Object(_))));
        }
        ParseOutcome::Rejected(received) => {
            let error = result.unwrap_err();
            let facts = error.facts();
            assert_eq!(
                (facts.status, facts.error_type, facts.param, facts.code),
                (400, "bad_request_error", Some(slot), "400")
            );
            assert!(error.to_string().contains(slot));
            assert!(error.to_string().contains(received));
        }
    }
}

#[test]
fn explicit_null_slot_stays_null_except_for_the_route_slot() {
    let data = body(vec![
        ("metadata", Json::Null),
        ("litellm_metadata", Json::Null),
    ]);
    let result =
        sanitize_request_metadata(&input(data, "/v1/chat/completions"), &settings()).unwrap();
    assert_eq!(
        result.data.get("metadata"),
        Some(&Json::Object(IndexMap::new()))
    );
    assert_eq!(result.data.get("litellm_metadata"), Some(&Json::Null));
}

#[rstest]
#[case::chat_completions("/v1/chat/completions", MetadataField::Metadata)]
#[case::messages("/v1/messages", MetadataField::LitellmMetadata)]
#[case::threads("/v1/threads/thread-1/runs", MetadataField::LitellmMetadata)]
#[case::assistants("/v1/assistants/asst-1", MetadataField::LitellmMetadata)]
#[case::batches("/v1/batches", MetadataField::LitellmMetadata)]
#[case::files("/v1/files", MetadataField::LitellmMetadata)]
#[case::responses("/v1/responses", MetadataField::LitellmMetadata)]
#[case::bedrock("/bedrock/model/invoke", MetadataField::LitellmMetadata)]
fn metadata_variable_name_follows_the_route(#[case] route: &str, #[case] expected: MetadataField) {
    assert_eq!(metadata_variable_name_for_route(route), expected);
}

#[test]
fn metadata_variable_name_from_data_checks_the_key() {
    let with_litellm_metadata = body(vec![("litellm_metadata", Json::Null)]);
    assert_eq!(
        metadata_variable_name_from_data(&with_litellm_metadata),
        MetadataField::LitellmMetadata
    );
    assert_eq!(
        metadata_variable_name_from_data(&body(vec![])),
        MetadataField::Metadata
    );
}

#[rstest]
#[case::chat_completions("/v1/chat/completions", true)]
#[case::messages("/v1/messages", true)]
#[case::mcp_route_is_not_inference("/mcp/tools/call", false)]
#[case::management_route("/health", false)]
fn inference_route_classification(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_llm_inference_route(route), expected);
}

#[derive(Clone, Copy, Debug)]
enum OptIn {
    KeyMetadata,
    TeamMetadata,
    TextTrue,
    FalseValue,
}

impl OptIn {
    fn principal(self) -> MetadataPrincipal {
        let value = match self {
            Self::KeyMetadata | Self::TeamMetadata => Json::Bool(true),
            Self::TextTrue => text("true"),
            Self::FalseValue => Json::Bool(false),
        };
        let flags = IndexMap::from([
            (ALLOW_CLIENT_PRICING_OVERRIDE_KEY.to_owned(), value.clone()),
            (
                ALLOW_CLIENT_MESSAGE_REDACTION_OPT_OUT_KEY.to_owned(),
                value.clone(),
            ),
            (ALLOW_CLIENT_MOCK_RESPONSE_KEY.to_owned(), value),
        ]);
        let mut principal = MetadataPrincipal::default();
        match self {
            Self::KeyMetadata | Self::TextTrue | Self::FalseValue => {
                principal.metadata = Some(flags);
            }
            Self::TeamMetadata => principal.team_metadata = Some(flags),
        }
        principal
    }

    const fn allows(self) -> bool {
        matches!(self, Self::KeyMetadata | Self::TeamMetadata)
    }
}

fn opt_in_data() -> RequestData {
    body(vec![
        ("input_cost_per_token", text("0.001")),
        ("turn_off_message_logging", Json::Bool(false)),
        ("dd_api_key", text("client-dd")),
        (
            "metadata",
            object(vec![
                ("model_info", object(vec![])),
                ("turn_off_message_logging", text("no")),
                ("attempted_fallbacks", Json::List(vec![])),
                (
                    "headers",
                    object(vec![
                        ("litellm-disable-message-redaction", text("1")),
                        ("x-keep", text("1")),
                    ]),
                ),
            ]),
        ),
        (
            "litellm_metadata",
            object(vec![("turn_off_message_logging", text("off"))]),
        ),
    ])
}

#[rstest]
#[case::key_metadata(OptIn::KeyMetadata)]
#[case::team_metadata(OptIn::TeamMetadata)]
#[case::text_true_is_not_a_flag(OptIn::TextTrue)]
#[case::false_is_not_a_flag(OptIn::FalseValue)]
fn admin_control_opt_in(#[case] opt_in: OptIn) {
    let (logs, _guard) = capture();
    let mut request = input_with_headers(
        opt_in_data(),
        &[("litellm-disable-message-redaction", "1")],
        "/v1/chat/completions",
    );
    request.principal = opt_in.principal();
    let result = sanitize_request_metadata(&request, &settings()).unwrap();
    let events = messages(&logs);
    if opt_in.allows() {
        assert_opt_in_kept(&result.data, &result.headers, &events);
    } else {
        assert_pricing_stripped(&result.data);
        assert_opt_out_stripped(&result.data, &result.headers);
    }
    assert_neutral_strips_apply(&result.data);
    assert_eq!(
        key_or_team_metadata_flag_is_true(&request.principal, ALLOW_CLIENT_MOCK_RESPONSE_KEY),
        opt_in.allows()
    );
}

fn assert_opt_in_kept(
    data: &RequestData,
    headers: &IndexMap<String, String>,
    events: &[(Level, String)],
) {
    assert_eq!(data.get("input_cost_per_token"), Some(&text("0.001")));
    assert_eq!(
        data.get("turn_off_message_logging"),
        Some(&Json::Bool(false))
    );
    let metadata = field(data, "metadata");
    assert!(metadata.contains_key("model_info"));
    assert_eq!(metadata.get("turn_off_message_logging"), Some(&text("no")));
    assert!(field(metadata, "headers").contains_key("litellm-disable-message-redaction"));
    assert!(headers.contains_key("litellm-disable-message-redaction"));
    assert!(
        !events
            .iter()
            .any(|(_level, message)| message.contains("pricing fields")
                || message.contains("message-redaction"))
    );
}

fn assert_neutral_strips_apply(data: &RequestData) {
    assert!(!data.contains_key("dd_api_key"));
    assert!(!field(data, "metadata").contains_key("attempted_fallbacks"));
    assert_eq!(
        field(field(data, "metadata"), "headers").get("x-keep"),
        Some(&text("1"))
    );
}

#[test]
fn promoted_trace_controls_keep_the_requester_order() {
    let requester = match object(vec![
        ("trace_id", text("requester-trace")),
        ("session_id", text("requester-session")),
        ("unrelated", text("value")),
    ]) {
        Json::Object(values) => values,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is an object")
        }
    };
    let litellm_metadata = match object(vec![("trace_id", text("proxy-trace"))]) {
        Json::Object(values) => values,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is an object")
        }
    };
    assert_eq!(
        promoted_trace_control_fields(&requester, &litellm_metadata),
        vec![("session_id".to_owned(), text("requester-session"))]
    );
}

fn principal_with_key(api_key: Option<&str>, via_virtual_key: bool) -> MetadataPrincipal {
    MetadataPrincipal {
        api_key: api_key.map(|value| SecretString::from(value.to_owned())),
        via_virtual_key,
        ..MetadataPrincipal::default()
    }
}

#[rstest]
#[case::master_key(
    Some("litellm_proxy_master_key"),
    true,
    Some("litellm_proxy_master_key")
)]
#[case::sha256_digest(
    None,
    true,
    Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
)]
#[case::custom_auth_key(Some("sk-custom"), true, None)]
#[case::uppercase_digest(
    Some("0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"),
    true,
    None
)]
#[case::unmarked_key(Some("litellm_proxy_master_key"), false, None)]
fn stampable_key_hash_cases(
    #[case] api_key: Option<&str>,
    #[case] via_virtual_key: bool,
    #[case] expected: Option<&str>,
) {
    let principal = principal_with_key(api_key, via_virtual_key);
    assert_eq!(stampable_key_hash(&principal).as_deref(), expected);
}

#[test]
fn sanitize_for_log_removes_line_breaks() {
    assert_eq!(sanitize_for_log(&text("a\r\nb\nc")), "abc");
    assert_eq!(sanitize_for_log(&Json::Number(Number::from(5))), "5");
    assert_eq!(
        sanitize_for_log(&object(vec![("a", text("b"))])),
        "{\"a\":\"b\"}"
    );
}

fn chain(headers: &[(&str, &str)]) -> Option<String> {
    let normalized: IndexMap<String, String> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    get_chain_id_from_headers(&normalized)
}

#[rstest]
#[case::explicit_trace_wins(&[("x-litellm-session-id", "session-1"), ("x-litellm-trace-id", "trace-1")], Some("trace-1"))]
#[case::explicit_session(&[("x-claude-code-session-id", "vendor-1-abcd"), ("x-litellm-session-id", "session-1")], Some("session-1"))]
#[case::generic_vendor(&[("x-claude-code-session-id", "e96634a3-fa28-4083"), ("x-session-id", "bare-1234")], Some("e96634a3-fa28-4083"))]
#[case::generic_first_in_order(&[("x-one-session-id", "first-1234"), ("x-two-session-id", "second-1234")], Some("first-1234"))]
#[case::bare_session(&[("X-Session-Id", "bare-1234")], Some("bare-1234"))]
#[case::codex_session(&[("user-agent", "codex_cli/1.0"), ("session-id", "codex-1234")], Some("codex-1234"))]
#[case::codex_requires_the_agent(&[("user-agent", "curl/8.0"), ("session-id", "codex-1234")], None)]
#[case::short_value_rejected(&[("x-session-id", "short")], None)]
#[case::invalid_value_rejected(&[("x-session-id", "not a session id")], None)]
#[case::empty_map(&[], None)]
#[case::empty_vendor_rejected(&[("x--session-id", "vendor-1234")], None)]
#[case::codex_prefix_needs_a_separator(&[("user-agent", "codex1.0"), ("session-id", "codex-1234")], None)]
fn chain_id_priority(#[case] headers: &[(&str, &str)], #[case] expected: Option<&str>) {
    assert_eq!(chain(headers).as_deref(), expected);
}

#[rstest]
#[case::nested_session(object(vec![("user_id", object(vec![("session_id", text("sess-1"))]))]), Some("sess-1"))]
#[case::nested_bad_session(object(vec![("user_id", object(vec![("session_id", text("bad value"))]))]), None)]
#[case::marker_in_user_id(object(vec![("user_id", text("user_session_abc-123"))]), Some("abc-123"))]
#[case::marker_with_invalid_tail(object(vec![("user_id", text("user_session_abc!"))]), None)]
#[case::no_marker(object(vec![("user_id", text("user"))]), None)]
#[case::not_an_object(text("user"), None)]
fn anthropic_session_id_cases(#[case] metadata: Json, #[case] expected: Option<&str>) {
    assert_eq!(
        anthropic_session_id_from_metadata(&metadata).as_deref(),
        expected
    );
}

#[test]
fn missing_session_id_omit_stage() {
    let mut data = body(vec![
        ("litellm_session_id", text("body-session")),
        (
            "metadata",
            object(vec![("litellm_session_id_omitted", Json::Bool(true))]),
        ),
        ("litellm_metadata", object(vec![])),
    ]);
    let policy = MissingSessionIdPolicy::Omit;
    apply_missing_session_id_policy(
        &mut data,
        MetadataField::LitellmMetadata,
        Some(&policy),
        true,
    );
    let slot = field(&data, "litellm_metadata");
    assert_eq!(
        slot.get("litellm_session_id_omitted"),
        Some(&Json::Bool(true))
    );
    assert_eq!(slot.get("session_id"), Some(&text("body-session")));
    assert!(!field(&data, "metadata").contains_key("litellm_session_id_omitted"));
}

#[test]
fn null_slot_session_still_copies_the_body_session() {
    let mut data = body(vec![
        ("litellm_session_id", text("body-session")),
        ("metadata", object(vec![])),
        ("litellm_metadata", object(vec![("session_id", Json::Null)])),
    ]);
    let policy = MissingSessionIdPolicy::Omit;
    apply_missing_session_id_policy(
        &mut data,
        MetadataField::LitellmMetadata,
        Some(&policy),
        true,
    );
    assert_eq!(
        field(&data, "litellm_metadata").get("session_id"),
        Some(&text("body-session"))
    );
}

#[derive(Clone, Copy, Debug)]
enum Policy {
    Omit,
    Generate,
    Reject,
    Unknown,
}

impl Policy {
    fn value(self) -> MissingSessionIdPolicy {
        match self {
            Self::Omit => MissingSessionIdPolicy::Omit,
            Self::Generate => MissingSessionIdPolicy::Generate,
            Self::Reject => MissingSessionIdPolicy::Reject,
            Self::Unknown => MissingSessionIdPolicy::Unrecognized("pause".to_owned()),
        }
    }
}

fn omit_guard_data(existing_session: Option<&str>, requester_session: bool) -> RequestData {
    let mut slot_metadata = IndexMap::new();
    slot_metadata.insert("litellm_session_id_omitted".to_owned(), Json::Bool(true));
    if let Some(session_id) = existing_session {
        slot_metadata.insert("session_id".to_owned(), text(session_id));
    }
    let mut requester = IndexMap::new();
    requester.insert("litellm_session_id_omitted".to_owned(), Json::Bool(true));
    if requester_session {
        requester.insert("session_id".to_owned(), text("requester-session"));
    }
    body(vec![
        ("litellm_session_id", text("body-session")),
        ("metadata", Json::Object(requester)),
        ("litellm_metadata", Json::Object(slot_metadata)),
    ])
}

fn session_in_slot(data: &RequestData) -> Option<&str> {
    match field(data, "litellm_metadata").get("session_id") {
        Some(Json::Text(value)) => Some(value.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[rstest]
#[case::existing_slot_session(
    Some(Policy::Omit),
    true,
    Some("slot-session"),
    true,
    Some("slot-session")
)]
#[case::requester_session_wins(Some(Policy::Omit), true, None, true, None)]
#[case::body_session_copied(Some(Policy::Omit), true, None, false, Some("body-session"))]
#[case::not_inference(Some(Policy::Omit), false, None, false, None)]
#[case::no_policy(None, true, None, false, None)]
#[case::generate_leaves_it_alone(Some(Policy::Generate), true, None, false, None)]
#[case::reject_leaves_it_alone(Some(Policy::Reject), true, None, false, None)]
#[case::unknown_leaves_it_alone(Some(Policy::Unknown), true, None, false, None)]
fn missing_session_id_omit_guards(
    #[case] policy: Option<Policy>,
    #[case] inference_route: bool,
    #[case] existing_session: Option<&str>,
    #[case] requester_session: bool,
    #[case] expected_session: Option<&str>,
) {
    let mut data = omit_guard_data(existing_session, requester_session);
    let resolved = policy.map(Policy::value);
    apply_missing_session_id_policy(
        &mut data,
        MetadataField::LitellmMetadata,
        resolved.as_ref(),
        inference_route,
    );
    let active = matches!(resolved, Some(MissingSessionIdPolicy::Omit)) && inference_route;
    assert_eq!(
        field(&data, "litellm_metadata").contains_key("litellm_session_id_omitted"),
        active
    );
    assert_eq!(
        session_in_slot(&data),
        if active {
            expected_session
        } else {
            existing_session
        }
    );
    assert!(!field(&data, "metadata").contains_key("litellm_session_id_omitted"));
}

fn guardrail_metadata(mode: &str) -> IndexMap<String, Json> {
    let entry = object(vec![
        ("guardrail_mode", text(mode)),
        ("guardrail_name", text("pii")),
    ]);
    match object(vec![(
        "standard_logging_guardrail_information",
        Json::List(vec![entry]),
    )]) {
        Json::Object(values) => values,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is an object")
        }
    }
}

fn snapshot_body(model: &str) -> RequestData {
    body(vec![
        ("model", text(model)),
        (
            "secret_fields",
            object(vec![("raw_headers", object(vec![]))]),
        ),
        (
            "headers",
            object(vec![("authorization", text("Bearer sk-1"))]),
        ),
        ("api_key", text("sk-1")),
        ("dd_api_key", text("dd-1")),
        ("langfuse_public_key", text("pk-1")),
        ("litellm_trusted_callback_vars", object(vec![])),
    ])
}

fn snapshot_data(metadata: Option<(&str, IndexMap<String, Json>)>) -> RequestData {
    let mut data = snapshot_body("gpt-4");
    data.insert(
        "proxy_server_request".to_owned(),
        object(vec![("url", text("/v1/chat/completions"))]),
    );
    if let Some((slot, values)) = metadata {
        data.insert(slot.to_owned(), Json::Object(values));
    }
    data
}

fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "gpt-4".to_owned(),
            call_type: "completion".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .expect("the fixture time is RFC 3339"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

fn stored_snapshot() -> Arc<GuardrailRequestSnapshot> {
    let body = snapshot_body("stale");
    let metadata = guardrail_metadata("pre_call");
    Arc::new(
        GuardrailRequestSnapshot::capture(&body, &metadata)
            .expect("a pre_call entry fingerprints the body"),
    )
}

#[derive(Clone, Copy, Debug)]
enum SnapshotCase {
    PreCallInLitellmMetadata,
    PostCallOnly,
    WithoutGuardrailsApplied,
    WithoutProxyServerRequest,
    MetadataSlotIsMetadata,
    LitellmMetadataShadowsMetadata,
}

impl SnapshotCase {
    const fn guardrails_applied(self) -> bool {
        !matches!(self, Self::WithoutGuardrailsApplied)
    }
}

fn snapshot_case_data(case: SnapshotCase) -> RequestData {
    match case {
        SnapshotCase::PreCallInLitellmMetadata | SnapshotCase::WithoutGuardrailsApplied => {
            snapshot_data(Some(("litellm_metadata", guardrail_metadata("pre_call"))))
        }
        SnapshotCase::PostCallOnly => {
            snapshot_data(Some(("litellm_metadata", guardrail_metadata("post_call"))))
        }
        SnapshotCase::WithoutProxyServerRequest => snapshot_body("gpt-4"),
        SnapshotCase::MetadataSlotIsMetadata => {
            snapshot_data(Some(("metadata", guardrail_metadata("pre_call"))))
        }
        SnapshotCase::LitellmMetadataShadowsMetadata => {
            let mut data = snapshot_data(Some(("metadata", guardrail_metadata("pre_call"))));
            data.insert(
                "litellm_metadata".to_owned(),
                Json::Object(guardrail_metadata("post_call")),
            );
            data
        }
    }
}

#[rstest]
#[case::pre_call_guarded(SnapshotCase::PreCallInLitellmMetadata)]
#[case::post_call_only(SnapshotCase::PostCallOnly)]
#[case::guardrails_not_applied(SnapshotCase::WithoutGuardrailsApplied)]
#[case::no_proxy_server_request(SnapshotCase::WithoutProxyServerRequest)]
#[case::metadata_slot_used(SnapshotCase::MetadataSlotIsMetadata)]
#[case::litellm_metadata_shadows_metadata(SnapshotCase::LitellmMetadataShadowsMetadata)]
fn guardrail_snapshot_stored(#[case] case: SnapshotCase) {
    let mut logging = logging_state();
    logging.shadow_eval_request_snapshot = Some(stored_snapshot());
    let mut data = snapshot_case_data(case);
    refresh_proxy_server_request_body_snapshot(
        &mut data,
        Some(&mut logging),
        case.guardrails_applied(),
    );
    match case {
        SnapshotCase::PreCallInLitellmMetadata | SnapshotCase::MetadataSlotIsMetadata => {
            let snapshot = logging.shadow_eval_request_snapshot.as_ref().unwrap();
            assert_eq!(snapshot.body.get("model"), Some(&text("gpt-4")));
            assert_excluded_from_snapshot(&snapshot.body);
            assert_eq!(written_body(&data), Some(&snapshot.body));
            let expected = request_guardrail_fingerprint(&guardrail_metadata("pre_call"));
            assert_eq!(Some(snapshot.fingerprint.clone()), expected);
        }
        SnapshotCase::PostCallOnly | SnapshotCase::LitellmMetadataShadowsMetadata => {
            assert!(logging.shadow_eval_request_snapshot.is_none());
            assert_eq!(
                written_body(&data).and_then(|body| body.get("model")),
                Some(&text("gpt-4"))
            );
        }
        SnapshotCase::WithoutGuardrailsApplied => {
            assert!(logging.shadow_eval_request_snapshot.is_none());
            let written = written_body(&data).expect("the refreshed body is written");
            assert_excluded_from_snapshot(written);
            assert_eq!(written.get("model"), Some(&text("gpt-4")));
        }
        SnapshotCase::WithoutProxyServerRequest => {
            assert!(logging.shadow_eval_request_snapshot.is_none());
            assert!(data.get("proxy_server_request").is_none());
        }
    }
}

fn written_body(data: &RequestData) -> Option<&IndexMap<String, Json>> {
    match data.get("proxy_server_request") {
        Some(Json::Object(request)) => match request.get("body") {
            Some(Json::Object(body)) => Some(body),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        },
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    }
}

fn assert_excluded_from_snapshot(body: &IndexMap<String, Json>) {
    let excluded = [
        "secret_fields",
        "proxy_server_request",
        "litellm_logging_obj",
        "headers",
        "api_key",
        "dd_api_key",
        "langfuse_public_key",
        "litellm_trusted_callback_vars",
    ];
    for name in excluded {
        assert!(!body.contains_key(name), "{name} leaked into the snapshot");
    }
}

#[test]
fn snapshot_refresh_without_logging_state_still_writes_the_body() {
    let mut data = snapshot_data(Some(("litellm_metadata", guardrail_metadata("pre_call"))));
    refresh_proxy_server_request_body_snapshot(&mut data, None, true);
    let written = written_body(&data).expect("the refreshed body is written");
    assert_excluded_from_snapshot(written);
    assert_eq!(written.get("model"), Some(&text("gpt-4")));
}
