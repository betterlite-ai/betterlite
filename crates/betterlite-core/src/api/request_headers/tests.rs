#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::CatalogProviderInference;
use betterlite_config::sections::request_headers::{Settings, UserHeaderMapping, UserHeaderRole};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_headers::cache_control::{CacheDirective, parse_cache_control};
use crate::api::request_headers::cleaning::REDACTED_HEADER_VALUE;
use crate::api::request_headers::error::RequestHeadersError;
use crate::api::request_headers::provider_headers::{
    ANTHROPIC_API_PROVIDERS, ANTHROPIC_OAUTH_PROVIDERS,
};
use crate::api::request_headers::trace_context::{
    session_id_from_baggage, trace_id_from_traceparent,
};
use crate::api::request_headers::{
    ForwardedHeaders, HeaderAuth, HeaderValue, RequestHeaders, RequestHeadersInput,
    RequestHeadersResult, SpanContext,
};
use crate::api::request_metadata_safety::metadata::MetadataField;
use crate::api::request_principal_metadata::PrincipalFacts;

const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0c9902b7-01";
const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

type Pairs = &'static [(&'static str, &'static str)];

fn header_map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn request_data(text: &str) -> RequestData {
    serde_json::from_str(text).unwrap()
}

fn input(pairs: &[(&str, &str)], data: &str) -> RequestHeadersInput {
    RequestHeadersInput {
        headers: header_map(pairs),
        data: request_data(data),
        metadata_slot: MetadataField::Metadata,
        auth: HeaderAuth::default(),
        principal: PrincipalFacts::default(),
        team_model_aliases: BTreeMap::new(),
        parent_span: None,
    }
}

fn prepare_with(
    settings: &Settings,
    otel_enabled: bool,
    limits: ApiLimits,
    request: RequestHeadersInput,
) -> Result<RequestHeadersResult, RequestHeadersError> {
    let inference = CatalogProviderInference::default();
    let aliases = BTreeMap::new();
    let model_access = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &inference,
        max_recurse_depth: 10,
    };
    RequestHeaders {
        settings,
        model_access: &model_access,
        limits,
        otel_enabled,
    }
    .prepare(request)
}

fn prepare(settings: &Settings, request: RequestHeadersInput) -> RequestHeadersResult {
    prepare_with(settings, false, ApiLimits::default(), request).unwrap()
}

fn exposed(headers: &ForwardedHeaders) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose().to_owned()))
        .collect();
    pairs.sort();
    pairs
}

fn sorted(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut owned: Vec<(String, String)> = header_map(pairs).into_iter().collect();
    owned.sort();
    owned
}

const CALLER_HEADERS: Pairs = &[
    ("authorization", "Bearer sk-proxy"),
    ("x-litellm-api-key", "sk-proxy"),
    ("x-api-key", "sk-provider"),
    ("X-Proxy-Key", "sk-proxy"),
    ("x-request-id", "req_1"),
    ("cookie", "session=1"),
    ("proxy-authorization", "Basic abc"),
    ("API-Key", "azure"),
    ("x-goog-api-key", "goog"),
    ("Ocp-Apim-Subscription-Key", "apim"),
    ("x-mcp-auth", "mcp"),
    ("x-mcp-servers", "github"),
    ("x-mcp-access-groups", "dev"),
];

fn auth(custom: Option<&str>, forward: bool, used: Option<&str>) -> HeaderAuth {
    HeaderAuth {
        litellm_key_header_name: custom.map(str::to_owned),
        forward_provider_auth: forward,
        authenticated_with_header: used.map(str::to_owned),
    }
}

#[rstest]
#[case::default_forwarding_off(
    CALLER_HEADERS,
    auth(Some("x-proxy-key"), false, Some("x-litellm-api-key")),
    &[("x-request-id", "req_1"), ("cookie", "session=1"), ("proxy-authorization", "Basic abc")],
    &[("x-request-id", "req_1"), ("cookie", REDACTED_HEADER_VALUE), ("proxy-authorization", REDACTED_HEADER_VALUE)]
)]
#[case::forwarding_on(
    CALLER_HEADERS,
    auth(Some("X-PROXY-KEY"), true, Some("x-litellm-api-key")),
    &[("x-api-key", "sk-provider"), ("x-request-id", "req_1"), ("cookie", "session=1"), ("proxy-authorization", "Basic abc"), ("API-Key", "azure"), ("x-goog-api-key", "goog"), ("Ocp-Apim-Subscription-Key", "apim"), ("x-mcp-auth", "mcp"), ("x-mcp-servers", "github"), ("x-mcp-access-groups", "dev")],
    &[("x-api-key", REDACTED_HEADER_VALUE), ("x-request-id", "req_1"), ("cookie", REDACTED_HEADER_VALUE), ("proxy-authorization", REDACTED_HEADER_VALUE), ("API-Key", REDACTED_HEADER_VALUE), ("x-goog-api-key", REDACTED_HEADER_VALUE), ("Ocp-Apim-Subscription-Key", REDACTED_HEADER_VALUE), ("x-mcp-auth", "mcp"), ("x-mcp-servers", "github"), ("x-mcp-access-groups", "dev")]
)]
#[case::forwarding_on_authenticated_with_x_api_key(
    &[("x-api-key", "sk-proxy"), ("x-request-id", "req_1")],
    auth(None, true, Some("X-API-Key")),
    &[("x-request-id", "req_1")],
    &[("x-request-id", "req_1")]
)]
#[case::forwarding_on_without_auth_header(
    &[("X-Api-Key", "sk-provider")],
    auth(None, true, None),
    &[("X-Api-Key", "sk-provider")],
    &[("X-Api-Key", REDACTED_HEADER_VALUE)]
)]
#[case::custom_key_shadows_provider_header(
    &[("x-goog-api-key", "sk-proxy"), ("api-key", "azure")],
    auth(Some("X-Goog-Api-Key"), true, Some("x-goog-api-key")),
    &[("api-key", "azure")],
    &[("api-key", REDACTED_HEADER_VALUE)]
)]
#[case::oauth_with_proxy_key_header(
    &[("Authorization", "Bearer sk-ant-oat-test"), ("x-litellm-api-key", "sk-proxy")],
    auth(None, false, Some("x-litellm-api-key")),
    &[("Authorization", "Bearer sk-ant-oat-test")],
    &[("Authorization", REDACTED_HEADER_VALUE)]
)]
#[case::oauth_without_auth_header(
    &[("authorization", "sk-ant-oat-test")],
    auth(None, false, None),
    &[("authorization", "sk-ant-oat-test")],
    &[("authorization", REDACTED_HEADER_VALUE)]
)]
#[case::oauth_used_as_proxy_key(
    &[("authorization", "Bearer sk-ant-oat-test")],
    auth(None, true, Some("Authorization")),
    &[],
    &[]
)]
fn forwarded_header_cleaning(
    #[case] headers: Pairs,
    #[case] header_auth: HeaderAuth,
    #[case] forwarded: Pairs,
    #[case] logged: Pairs,
) {
    let mut request = input(headers, "{}");
    request.auth = header_auth;
    let result = prepare(&Settings::default(), request);
    assert_eq!(exposed(&result.forwarded), sorted(forwarded));
    let mut logged_pairs: Vec<(String, String)> = result.logged.into_iter().collect();
    logged_pairs.sort();
    assert_eq!(logged_pairs, sorted(logged));
}

#[rstest]
#[case::opted_in(true, true, &[("traceparent", TRACEPARENT)], "{}", Some(r#"{"traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0c9902b7-01"}"#))]
#[case::header_case(true, true, &[("TraceParent", "00-a")], r#"{"extra_headers": {"x-a": "1"}}"#, Some(r#"{"x-a": "1", "traceparent": "00-a"}"#))]
#[case::keeps_request_value(true, true, &[("traceparent", "00-a")], r#"{"extra_headers": {"traceparent": "mine"}}"#, Some(r#"{"traceparent": "mine"}"#))]
#[case::null_extra_headers(true, true, &[("traceparent", "00-a")], r#"{"extra_headers": null}"#, Some(r#"{"traceparent": "00-a"}"#))]
#[case::non_object_extra_headers(true, true, &[("traceparent", "00-a")], r#"{"extra_headers": "text"}"#, Some(r#""text""#))]
#[case::not_opted_in(false, true, &[("traceparent", "00-a")], "{}", None)]
#[case::no_otel_logger(true, false, &[("traceparent", "00-a")], "{}", None)]
#[case::no_header(true, true, &[("x-request-id", "req_1")], "{}", None)]
fn traceparent_opt_in(
    #[case] forward: bool,
    #[case] otel_enabled: bool,
    #[case] headers: Pairs,
    #[case] data: &str,
    #[case] extra_headers: Option<&str>,
) {
    let settings = configured(|settings| settings.forward_traceparent = forward);
    let result = prepare_with(
        &settings,
        otel_enabled,
        ApiLimits::default(),
        input(headers, data),
    )
    .unwrap();
    let want = extra_headers.map(|text| serde_json::from_str(text).unwrap());
    assert_eq!(result.data.get("extra_headers"), want.as_ref());
}

fn mapping(header_name: &str, litellm_user_role: UserHeaderRole) -> UserHeaderMapping {
    UserHeaderMapping {
        header_name: header_name.to_owned(),
        litellm_user_role,
    }
}

#[rstest]
#[case::internal_user_mapping(
    vec![mapping("X-End-User", UserHeaderRole::Customer), mapping("X-OpenWebUI-User-Id", UserHeaderRole::InternalUser)],
    None,
    &[("x-openwebui-user-id", "user-7"), ("x-end-user", "customer-1")],
    None,
    Some("user-7")
)]
#[case::empty_mapping_header_skipped(
    vec![mapping("", UserHeaderRole::InternalUser), mapping("X-User-Id", UserHeaderRole::InternalUser)],
    None,
    &[("x-user-id", "user-8")],
    None,
    Some("user-8")
)]
#[case::empty_header_value(
    vec![mapping("X-User-Id", UserHeaderRole::InternalUser)],
    None,
    &[("x-user-id", "")],
    None,
    None
)]
#[case::missing_header(vec![mapping("X-User-Id", UserHeaderRole::InternalUser)], None, &[], None, None)]
#[case::user_header_name(Vec::new(), Some("X-User"), &[("x-user", "end-1"), ("X-User", "end-2")], Some("end-1"), None)]
#[case::user_header_name_empty(Vec::new(), Some(""), &[("", "x")], None, None)]
#[case::user_header_name_missing(Vec::new(), Some("X-User"), &[("x-other", "end-1")], None, None)]
fn user_header_mapping(
    #[case] user_header_mappings: Vec<UserHeaderMapping>,
    #[case] user_header_name: Option<&str>,
    #[case] headers: Pairs,
    #[case] user: Option<&str>,
    #[case] internal_user_id: Option<&str>,
) {
    let settings = configured(|settings| {
        settings.user_header_mappings = user_header_mappings;
        settings.user_header_name = user_header_name.map(str::to_owned);
    });
    let result = prepare(&settings, input(headers, "{}"));
    assert_eq!(result.user.as_deref(), user);
    assert_eq!(result.internal_user_id.as_deref(), internal_user_id);
}

#[derive(Debug, Default)]
struct Backend {
    headers: Option<Pairs>,
    organization: Option<&'static str>,
    timeout: Option<f64>,
    stream_timeout: Option<f64>,
    client_side_timeout: bool,
    num_retries: Option<i64>,
    keepalive_seconds: Option<f64>,
}

fn backend(change: fn(&mut Backend)) -> Backend {
    let mut want = Backend::default();
    change(&mut want);
    want
}

fn configured<F>(change: F) -> Settings
where
    F: FnOnce(&mut Settings),
{
    let mut settings = Settings::default();
    change(&mut settings);
    settings
}

fn model_group(models: &[&str]) -> Settings {
    configured(|settings| {
        settings.model_group_forwarding =
            Some(models.iter().map(|model| (*model).to_owned()).collect());
    })
}

#[rstest]
#[case::nothing_set(Settings::default(), &[("OpenAI-Organization", "org-1"), ("x-a", "1")], "{}", Backend::default())]
#[case::transport_headers(
    configured(|settings| {
        settings.forward_client_headers = true;
        settings.forward_openai_org_id = true;
    }),
    &[("x-litellm-timeout", " 90 "), ("x-litellm-stream-timeout", "1e1"), ("x-litellm-num-retries", "3"), ("x-litellm-keepalive-seconds", "0.5"), ("OpenAI-Organization", "org-1"), ("x-stainless-os", "mac"), ("anthropic-beta-extra", "b"), ("accept", "*/*")],
    "{}",
    Backend { headers: Some(&[("x-litellm-timeout", " 90 "), ("x-litellm-stream-timeout", "1e1"), ("x-litellm-num-retries", "3"), ("x-litellm-keepalive-seconds", "0.5"), ("anthropic-beta-extra", "b")]), organization: Some("org-1"), timeout: Some(90.0_f64), stream_timeout: Some(10.0_f64), client_side_timeout: true, num_retries: Some(3), keepalive_seconds: Some(0.5_f64) }
)]
#[case::stream_timeout_alone(Settings::default(), &[("x-litellm-stream-timeout", "5")], "{}", backend(|want| { want.stream_timeout = Some(5.0_f64); want.client_side_timeout = true; }))]
#[case::body_timeout(Settings::default(), &[], r#"{"request_timeout": 3}"#, backend(|want| want.client_side_timeout = true))]
#[case::body_stream_timeout(Settings::default(), &[], r#"{"stream_timeout": 3}"#, backend(|want| want.client_side_timeout = true))]
#[case::null_body_timeout(Settings::default(), &[], r#"{"timeout": null}"#, Backend::default())]
#[case::no_forwardable_headers(configured(|settings| settings.forward_client_headers = true), &[("accept", "*/*")], "{}", Backend::default())]
#[case::model_group_match(model_group(&["gpt-4o"]), &[("x-a", "1")], r#"{"model": "gpt-4o"}"#, backend(|want| want.headers = Some(&[("x-a", "1")])))]
#[case::model_group_other_model(model_group(&["gpt-4o"]), &[("x-a", "1")], r#"{"model": "claude"}"#, Backend::default())]
#[case::model_group_every_model(model_group(&[]), &[("x-a", "1")], r#"{"model": "claude"}"#, backend(|want| want.headers = Some(&[("x-a", "1")])))]
#[case::model_group_no_model(model_group(&[]), &[("x-a", "1")], "{}", Backend::default())]
#[case::model_group_no_headers(model_group(&[]), &[("accept", "*/*")], r#"{"model": "claude"}"#, Backend::default())]
fn backend_call_arguments(
    #[case] settings: Settings,
    #[case] headers: Pairs,
    #[case] data: &str,
    #[case] expected: Backend,
) {
    let backend = prepare(&settings, input(headers, data)).backend;
    assert_eq!(
        backend.headers.as_ref().map(exposed),
        expected.headers.map(sorted)
    );
    assert_eq!(backend.organization.as_deref(), expected.organization);
    assert_eq!(backend.timeout, expected.timeout);
    assert_eq!(backend.stream_timeout, expected.stream_timeout);
    assert_eq!(backend.client_side_timeout, expected.client_side_timeout);
    assert_eq!(backend.num_retries, expected.num_retries);
    assert_eq!(backend.keepalive_seconds, expected.keepalive_seconds);
}

#[test]
fn backend_call_arguments_add_user_information() {
    let mut settings = model_group(&["gpt-4o"]);
    settings.add_user_information = true;
    let mut request = input(&[("x-a", "1")], r#"{"model": "team-gpt"}"#);
    request.principal.api_key_hash = Some("hash-1".to_owned());
    request.principal.spend = Decimal::new(15, 1);
    request.principal.team_id = Some("team-1".to_owned());
    request.team_model_aliases = BTreeMap::from([("team-gpt".to_owned(), "gpt-4o".to_owned())]);
    let backend = prepare(&settings, request).backend;
    let want = sorted(&[
        ("x-a", "1"),
        ("x-litellm-user_api_key_hash", "hash-1"),
        ("x-litellm-user_api_key_spend", "1.5"),
        ("x-litellm-user_api_key_team_id", "team-1"),
        ("x-litellm-user_api_key_auth_metadata", "{}"),
    ]);
    assert_eq!(backend.headers.as_ref().map(exposed), Some(want));
}

#[rstest]
#[case::timeout("x-litellm-timeout", "ninety")]
#[case::stream_timeout("x-litellm-stream-timeout", "")]
#[case::keepalive("x-litellm-keepalive-seconds", "1_0")]
fn backend_call_arguments_reject_numbers(#[case] header: &'static str, #[case] value: &str) {
    let failed = prepare_with(
        &Settings::default(),
        false,
        ApiLimits::default(),
        input(&[(header, value)], "{}"),
    );
    assert!(
        matches!(failed, Err(RequestHeadersError::Number { header: named, .. }) if named == header)
    );
}

#[test]
fn backend_call_arguments_reject_retries() {
    let failed = prepare_with(
        &Settings::default(),
        false,
        ApiLimits::default(),
        input(&[("x-litellm-num-retries", "3.0")], "{}"),
    );
    assert!(matches!(
        failed,
        Err(RequestHeadersError::Integer {
            header: "x-litellm-num-retries",
            ..
        })
    ));
}

#[rstest]
#[case::spaced(&format!(" {TRACEPARENT} "), Some(TRACE_ID))]
#[case::upper_case(
    "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0C9902B7-01",
    Some(TRACE_ID)
)]
#[case::zero_trace("00-00000000000000000000000000000000-00f067aa0c9902b7-01", None)]
#[case::short_trace("00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0c9902b7-01", None)]
#[case::extra_field("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0c9902b7-01-00", None)]
#[case::three_fields("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0c9902b7", None)]
#[case::not_hex("00-4bf92f3577b34da6a3ce929d0e0e473g-00f067aa0c9902b7-01", None)]
fn parses_traceparent(#[case] traceparent: &str, #[case] trace_id: Option<&str>) {
    assert_eq!(trace_id_from_traceparent(traceparent).as_deref(), trace_id);
}

#[rstest]
#[case::session(" user.id=42 , session.id = abc-123 ", Some("abc-123"))]
#[case::empty_value("session.id= ,session.id=second", Some("second"))]
#[case::no_equals("session.id", None)]
#[case::other_key("sessionid=abc", None)]
fn reads_baggage_session(#[case] baggage: &str, #[case] session_id: Option<&str>) {
    assert_eq!(session_id_from_baggage(baggage).as_deref(), session_id);
}

#[rstest]
#[case::chain_header(
    &[("x-litellm-trace-id", "chain-1"), ("traceparent", TRACEPARENT), ("baggage", "session.id=bag-1")],
    r#"{"metadata": {"a": 1}}"#,
    r#"{"metadata": {"a": 1, "trace_id": "chain-1", "session_id": "chain-1"}, "litellm_session_id": "chain-1", "litellm_trace_id": "chain-1"}"#
)]
#[case::w3c_fallback(
    &[("Traceparent", "bad"), ("traceparent", TRACEPARENT), ("Baggage", "session.id=bag-1"), ("x-litellm-agent-id", "agent-1"), ("x-litellm-spend-logs-metadata", r#"{"team": "a"}"#)],
    r#"{"metadata": {}}"#,
    r#"{"metadata": {"spend_logs_metadata": {"team": "a"}, "agent_id": "agent-1", "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736", "session_id": "bag-1"}, "litellm_trace_id": "4bf92f3577b34da6a3ce929d0e0e4736", "litellm_session_id": "bag-1"}"#
)]
#[case::request_ids_win(
    &[("traceparent", TRACEPARENT), ("baggage", "session.id=bag-1"), ("x-litellm-agent-id", ""), ("x-litellm-spend-logs-metadata", "null")],
    r#"{"metadata": {}, "litellm_trace_id": null, "litellm_session_id": "mine"}"#,
    r#"{"metadata": {}, "litellm_trace_id": null, "litellm_session_id": "mine"}"#
)]
#[case::anthropic_session(
    &[("x-litellm-spend-logs-metadata", "not json")],
    r#"{"metadata": {"user_id": {"session_id": "sess-12345"}}}"#,
    r#"{"metadata": {"user_id": "sess-12345", "session_id": "sess-12345"}, "litellm_session_id": "sess-12345"}"#
)]
#[case::anthropic_session_text(
    &[],
    r#"{"metadata": {"user_id": "user_1_account__session_sess-9"}}"#,
    r#"{"metadata": {"user_id": "user_1_account__session_sess-9", "session_id": "sess-9"}, "litellm_session_id": "sess-9"}"#
)]
#[case::non_object_metadata_slot(&[("x-litellm-agent-id", "agent-1")], r#"{"metadata": "text"}"#, r#"{"metadata": "text"}"#)]
#[case::missing_metadata_slot(&[("x-litellm-agent-id", "agent-1")], "{}", "{}")]
fn header_metadata(#[case] headers: Pairs, #[case] data: &str, #[case] expected: &str) {
    let result = prepare(&Settings::default(), input(headers, data));
    assert_eq!(result.data, request_data(expected));
}

#[rstest]
#[case::within_limit(13, r#"{"metadata": {"spend_logs_metadata": {"a": 1}}}"#)]
#[case::over_limit(12, r#"{"metadata": {}}"#)]
fn header_metadata_json_limit(#[case] max_json_bytes: usize, #[case] expected: &str) {
    let limits = ApiLimits {
        max_json_bytes,
        max_event_bytes: 1024,
        max_items: 64,
    };
    let request = input(
        &[("x-litellm-spend-logs-metadata", r#"{"a":    1  }"#)],
        r#"{"metadata": {}}"#,
    );
    let result = prepare_with(&Settings::default(), false, limits, request).unwrap();
    assert_eq!(result.data, request_data(expected));
}

#[rstest]
#[case::stamps_slot(MetadataField::LitellmMetadata, Some(SpanContext { trace_id: 255, is_valid: true }), r#"{"litellm_metadata": {}, "metadata": {}}"#, r#"{"litellm_metadata": {"trace_id": "000000000000000000000000000000ff"}, "metadata": {}, "litellm_trace_id": "000000000000000000000000000000ff"}"#)]
#[case::invalid_span(MetadataField::Metadata, Some(SpanContext { trace_id: 255, is_valid: false }), r#"{"metadata": {}}"#, r#"{"metadata": {}}"#)]
#[case::no_span(
    MetadataField::Metadata,
    None,
    r#"{"metadata": {}}"#,
    r#"{"metadata": {}}"#
)]
#[case::request_trace_id(MetadataField::Metadata, Some(SpanContext { trace_id: 1, is_valid: true }), r#"{"litellm_trace_id": "mine"}"#, r#"{"litellm_trace_id": "mine"}"#)]
#[case::slot_trace_id(MetadataField::LitellmMetadata, Some(SpanContext { trace_id: 1, is_valid: true }), r#"{"litellm_metadata": {"trace_id": "t"}}"#, r#"{"litellm_metadata": {"trace_id": "t"}}"#)]
#[case::requester_trace_id(MetadataField::LitellmMetadata, Some(SpanContext { trace_id: 1, is_valid: true }), r#"{"metadata": {"trace_id": "t"}}"#, r#"{"metadata": {"trace_id": "t"}}"#)]
#[case::null_ids(MetadataField::Metadata, Some(SpanContext { trace_id: 1, is_valid: true }), r#"{"litellm_trace_id": null, "metadata": {"trace_id": null}}"#, r#"{"litellm_trace_id": "00000000000000000000000000000001", "metadata": {"trace_id": "00000000000000000000000000000001"}}"#)]
fn stamps_otel_trace_id(
    #[case] slot: MetadataField,
    #[case] parent_span: Option<SpanContext>,
    #[case] data: &str,
    #[case] expected: &str,
) {
    let mut request = input(&[], data);
    request.metadata_slot = slot;
    request.parent_span = parent_span;
    let result = prepare(&Settings::default(), request);
    assert_eq!(result.data, request_data(expected));
}

#[rstest]
#[case::both(&[("anthropic-version", "2023-06-01"), ("anthropic-beta", "b"), ("Authorization", "Bearer sk-ant-oat-1")], vec![(ANTHROPIC_API_PROVIDERS, sorted(&[("anthropic-version", "2023-06-01"), ("anthropic-beta", "b")])), (ANTHROPIC_OAUTH_PROVIDERS, sorted(&[("Authorization", "Bearer sk-ant-oat-1")]))], true)]
#[case::api_headers_only(&[("anthropic-beta", "b"), ("Anthropic-Version", "x")], vec![(ANTHROPIC_API_PROVIDERS, sorted(&[("anthropic-beta", "b")]))], false)]
#[case::none(&[("x-a", "1")], Vec::new(), false)]
#[case::oauth_value_in_other_header(&[("x-token", "Bearer sk-ant-oat-1")], Vec::new(), false)]
fn provider_specific_headers(
    #[case] headers: Pairs,
    #[case] scoped: Vec<(&str, Vec<(String, String)>)>,
    #[case] used_client_oauth_token: bool,
) {
    let provider = prepare(&Settings::default(), input(headers, "{}")).provider;
    let got: Vec<(&str, Vec<(String, String)>)> = provider
        .scoped
        .iter()
        .map(|header| (header.custom_llm_provider, exposed(&header.extra_headers)))
        .collect();
    assert_eq!(got, scoped);
    assert_eq!(provider.used_client_oauth_token, used_client_oauth_token);
    let credential = provider
        .scoped
        .iter()
        .flat_map(|header| header.extra_headers.values())
        .any(|value| matches!(value, HeaderValue::Credential(_)));
    assert_eq!(credential, used_client_oauth_token);
}

#[rstest]
#[case::directives("no-cache, max-age=0, =x, a=", &[("no-cache", None), ("max-age", Some("0")), ("", Some("x")), ("a", Some(""))])]
#[case::comma_without_space("no-store,max-age=5", &[("no-store,max-age", Some("5"))])]
#[case::empty("", &[("", None)])]
fn parses_cache_control(#[case] text: &str, #[case] expected: &[(&str, Option<&str>)]) {
    let want: IndexMap<String, CacheDirective> = expected
        .iter()
        .map(|(key, value)| {
            let directive = value.map_or(CacheDirective::Flag, |given| {
                CacheDirective::Value(given.to_owned())
            });
            ((*key).to_owned(), directive)
        })
        .collect();
    assert_eq!(parse_cache_control(text).unwrap(), want);
}

#[test]
fn rejects_cache_control_with_two_equals() {
    let failed = parse_cache_control("max-age=1=2");
    assert!(
        matches!(failed, Err(RequestHeadersError::CacheControl { directive }) if directive == "max-age=1=2")
    );
}
