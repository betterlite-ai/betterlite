#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{destination_capable_backends, destination_for};
use crate::otel::config::destination::{OtelDestination, SpanScope};

const US_CLOUD: &str = "https://us.cloud.langfuse.com/api/public/otel";

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn allowlist(entries: &[&str]) -> Vec<AllowedDestination> {
    entries
        .iter()
        .filter_map(|entry| AllowedDestination::parse(entry))
        .collect()
}

fn params(entries: &[(&str, &str)]) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
            .collect::<IndexMap<String, Json>>(),
    )
}

fn langfuse_credentials() -> [(&'static str, &'static str); 2] {
    [("langfuse_public_key", "pk"), ("langfuse_secret_key", "sk")]
}

fn langfuse_params(extra: &[(&str, &str)]) -> Json {
    let mut entries = langfuse_credentials().to_vec();
    entries.extend_from_slice(extra);
    params(&entries)
}

fn basic_header(user: &str) -> String {
    format!("Basic {}", STANDARD.encode(user))
}

fn langfuse_basic() -> String {
    basic_header("pk:sk")
}

#[test]
fn destination_capable_backends_are_exactly_the_four() {
    assert_eq!(
        destination_capable_backends(),
        BTreeSet::from(["arize", "langfuse_otel", "newrelic", "weave_otel"])
    );
}

fn destination(
    variables: &[(&str, &str)],
    allowed: &[&str],
    callback_name: &str,
    given: &Json,
) -> Option<OtelDestination> {
    destination_for(
        &environment(variables),
        &allowlist(allowed),
        callback_name,
        given,
        None,
    )
}

#[rstest]
#[case::signoz("signoz")]
#[case::agentops("agentops")]
#[case::arize_phoenix("arize_phoenix")]
#[case::unknown("datadog")]
fn only_capable_backends_resolve_a_destination(#[case] callback_name: &str) {
    let given = params(&[("api_key", "key-1"), ("signoz_ingestion_key", "key-1")]);
    assert!(destination(&[], &[], callback_name, &given).is_none());
}

#[test]
fn signoz_has_headers_and_an_endpoint_but_no_destination() {
    let given = params(&[
        (
            "signoz_ingestion_endpoint",
            "https://collector.example/v1/traces",
        ),
        ("signoz_ingestion_key", "key-1"),
    ]);
    assert!(destination(&[], &["collector.example"], "signoz", &given).is_none());
}

#[test]
fn langfuse_defaults_to_the_us_cloud_endpoint() {
    let resolved = destination(&[], &[], "langfuse_otel", &langfuse_params(&[]))
        .expect("the tenant has credentials");
    let fields = resolved.as_ref();
    assert_eq!(fields.endpoint, US_CLOUD);
    assert_eq!(
        fields.headers,
        BTreeMap::from([
            ("Authorization".to_owned(), langfuse_basic()),
            ("x-langfuse-ingestion-version".to_owned(), "4".to_owned()),
        ])
    );
    assert_eq!(fields.callback_name.as_deref(), Some("langfuse_otel"));
    assert_eq!(fields.protocol, None);
    assert_eq!(fields.span_scope, SpanScope::Full);
}

#[rstest]
#[case::otel_host_wins(&[("LANGFUSE_OTEL_HOST", "otel.example"), ("LANGFUSE_HOST", "langfuse.example")], "otel.example")]
#[case::langfuse_host(&[("LANGFUSE_HOST", "langfuse.example")], "langfuse.example")]
#[case::empty_otel_host(&[("LANGFUSE_OTEL_HOST", ""), ("LANGFUSE_HOST", "langfuse.example")], "langfuse.example")]
fn langfuse_uses_the_operator_host_without_checking_it(
    #[case] variables: &[(&str, &str)],
    #[case] host: &str,
) {
    let resolved = destination(variables, &[], "langfuse_otel", &langfuse_params(&[]))
        .expect("the operator host is never checked");
    assert_eq!(
        resolved.as_ref().endpoint,
        format!("https://{host}/api/public/otel")
    );
}

#[rstest]
#[case::scheme_kept("https://tenant.example", "https://tenant.example/api/public/otel")]
#[case::scheme_added("tenant.example", "https://tenant.example/api/public/otel")]
#[case::trailing_slash("tenant.example/", "https://tenant.example/api/public/otel")]
fn langfuse_normalizes_the_tenant_host(#[case] host: &str, #[case] endpoint: &str) {
    let given = langfuse_params(&[("langfuse_host", host)]);
    let resolved = destination(&[], &["tenant.example"], "langfuse_otel", &given)
        .expect("the tenant host is allowlisted");
    assert_eq!(resolved.as_ref().endpoint, endpoint);
}

#[test]
fn langfuse_refuses_a_tenant_host_outside_the_allowlist() {
    let given = langfuse_params(&[("langfuse_host", "tenant.example")]);
    assert!(destination(&[], &[], "langfuse_otel", &given).is_none());
    assert!(destination(&[], &["other.example"], "langfuse_otel", &given).is_none());
}

#[test]
fn langfuse_checks_the_allowlist_against_the_final_endpoint() {
    let given = langfuse_params(&[("langfuse_host", "tenant.example")]);
    let resolved = destination(&[], &["https://tenant.example"], "langfuse_otel", &given)
        .expect("the scheme-specific entry matches");
    assert_eq!(
        resolved.as_ref().endpoint,
        "https://tenant.example/api/public/otel"
    );
}

#[rstest]
#[case::missing_secret(&[("langfuse_public_key", "pk")])]
#[case::missing_public(&[("langfuse_secret_key", "sk")])]
fn langfuse_needs_the_authorization_header(#[case] entries: &[(&str, &str)]) {
    assert!(destination(&[], &[], "langfuse_otel", &params(entries)).is_none());
}

#[rstest]
#[case::llm_only("llm_only", SpanScope::LlmOnly)]
#[case::full("full", SpanScope::Full)]
#[case::upper_case("LLM_ONLY", SpanScope::Full)]
#[case::unknown("everything", SpanScope::Full)]
#[case::empty("", SpanScope::Full)]
fn langfuse_span_scope_is_llm_only_for_the_exact_value(
    #[case] scope: &str,
    #[case] expected: SpanScope,
) {
    let given = langfuse_params(&[("langfuse_span_scope", scope)]);
    let resolved = destination(&[], &[], "langfuse_otel", &given).expect("credentials are set");
    assert_eq!(resolved.as_ref().span_scope, expected);
}

#[test]
fn other_backends_ignore_the_langfuse_span_scope() {
    let given = params(&[
        ("newrelic_api_key", "key-1"),
        ("langfuse_span_scope", "llm_only"),
    ]);
    let resolved = destination(&[], &[], "newrelic", &given).expect("the key is set");
    assert_eq!(resolved.as_ref().span_scope, SpanScope::Full);
}

fn arize_params() -> Json {
    params(&[("arize_space_id", "space-1"), ("arize_api_key", "key-1")])
}

#[rstest]
#[case::default_grpc(&[], "https://otlp.arize.com/v1", "otlp_grpc")]
#[case::grpc_endpoint(&[("ARIZE_ENDPOINT", "https://grpc.example/v1")], "https://grpc.example/v1", "otlp_grpc")]
#[case::http_endpoint(&[("ARIZE_HTTP_ENDPOINT", "https://http.example/v1")], "https://http.example/v1", "otlp_http")]
fn arize_comes_from_the_operator_environment(
    #[case] variables: &[(&str, &str)],
    #[case] endpoint: &str,
    #[case] protocol: &str,
) {
    let resolved = destination(variables, &[], "arize", &arize_params()).expect("headers are set");
    let fields = resolved.as_ref();
    assert_eq!(fields.endpoint, endpoint);
    assert_eq!(fields.protocol.as_deref(), Some(protocol));
    assert_eq!(
        fields.headers,
        BTreeMap::from([
            ("api_key".to_owned(), "key-1".to_owned()),
            ("arize-space-id".to_owned(), "space-1".to_owned()),
        ])
    );
}

#[test]
fn arize_ignores_request_parameters_for_the_endpoint() {
    let given = params(&[
        ("arize_space_id", "space-1"),
        ("arize_api_key", "key-1"),
        ("arize_endpoint", "https://tenant.example/v1"),
    ]);
    let resolved = destination(&[], &[], "arize", &given).expect("headers are set");
    assert_eq!(resolved.as_ref().endpoint, "https://otlp.arize.com/v1");
}

#[rstest]
#[case::missing_space_id(&[("arize_api_key", "key-1")])]
#[case::missing_api_key(&[("arize_space_id", "space-1")])]
#[case::space_id_only(&[("arize_space_id", "space-1")])]
fn arize_needs_both_headers(#[case] entries: &[(&str, &str)]) {
    assert!(destination(&[], &[], "arize", &params(entries)).is_none());
}

fn weave_params() -> Json {
    params(&[
        ("wandb_api_key", "key-1"),
        ("weave_project_id", "entity/project"),
    ])
}

#[rstest]
#[case::default(&[], "https://trace.wandb.ai/otel/v1/traces")]
#[case::host(&[("WANDB_HOST", "weave.example")], "https://weave.example/otel/v1/traces")]
#[case::host_with_scheme(&[("WANDB_HOST", "http://weave.example")], "http://weave.example/otel/v1/traces")]
#[case::host_with_slash(&[("WANDB_HOST", "weave.example/")], "https://weave.example/otel/v1/traces")]
#[case::empty_host(&[("WANDB_HOST", "")], "https://trace.wandb.ai/otel/v1/traces")]
fn weave_uses_the_wandb_host(#[case] variables: &[(&str, &str)], #[case] endpoint: &str) {
    let resolved =
        destination(variables, &[], "weave_otel", &weave_params()).expect("headers are set");
    let fields = resolved.as_ref();
    assert_eq!(fields.endpoint, endpoint);
    assert_eq!(fields.protocol, None);
    assert_eq!(
        fields.headers,
        BTreeMap::from([
            ("Authorization".to_owned(), basic_header("api:key-1")),
            ("project_id".to_owned(), "entity/project".to_owned()),
        ])
    );
}

#[rstest]
#[case::missing_key(&[("weave_project_id", "entity/project")])]
#[case::missing_project(&[("wandb_api_key", "key-1")])]
fn weave_needs_both_headers(#[case] entries: &[(&str, &str)]) {
    assert!(destination(&[], &[], "weave_otel", &params(entries)).is_none());
}

#[rstest]
#[case::unset(&[], "https://otlp.nr-data.net")]
#[case::eu(&[("newrelic_region", "eu")], "https://otlp.eu01.nr-data.net")]
#[case::unknown(&[("newrelic_region", "mars")], "https://otlp.nr-data.net")]
fn newrelic_uses_the_team_region(#[case] entries: &[(&str, &str)], #[case] endpoint: &str) {
    let mut all = vec![("newrelic_api_key", "key-1")];
    all.extend_from_slice(entries);
    let resolved = destination(&[], &[], "newrelic", &params(&all)).expect("the key is set");
    let fields = resolved.as_ref();
    assert_eq!(fields.endpoint, endpoint);
    assert_eq!(fields.protocol, None);
    assert_eq!(
        fields.headers,
        BTreeMap::from([("api-key".to_owned(), "key-1".to_owned())])
    );
}

#[test]
fn newrelic_needs_the_api_key_header() {
    assert!(destination(&[], &[], "newrelic", &params(&[("newrelic_region", "eu")])).is_none());
}

#[rstest]
#[case::given(Some("service-1"), 1)]
#[case::empty(Some(""), 0)]
#[case::missing(None, 0)]
fn service_name_becomes_a_resource_attribute(
    #[case] service_name: Option<&str>,
    #[case] expected_attributes: usize,
) {
    let resolved = destination_for(
        &environment(&[]),
        &allowlist(&[]),
        "newrelic",
        &params(&[("newrelic_api_key", "key-1")]),
        service_name,
    )
    .expect("the key is set");
    let attributes = &resolved.as_ref().resource_attributes;
    assert_eq!(attributes.len(), expected_attributes);
    assert_eq!(
        attributes.get("service.name").map(String::as_str),
        service_name.filter(|name| !name.is_empty())
    );
}

#[test]
fn the_callback_name_is_recorded_on_the_destination() {
    let resolved = destination(
        &[],
        &[],
        "newrelic",
        &params(&[("newrelic_api_key", "key-1")]),
    )
    .expect("the key is set");
    assert_eq!(resolved.as_ref().callback_name.as_deref(), Some("newrelic"));
}
