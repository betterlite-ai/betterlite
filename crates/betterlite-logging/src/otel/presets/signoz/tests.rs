#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use rstest::rstest;

use super::{
    SIGNOZ_INGESTION_ENDPOINT_ENV, SignozPreset, signoz_dynamic_endpoint, signoz_dynamic_headers,
};
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::presets::base::Preset as _;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment(variables)
}

fn allowlist(entries: &[&str]) -> Vec<AllowedDestination> {
    entries
        .iter()
        .filter_map(|entry| AllowedDestination::parse(entry))
        .collect()
}

fn base_settings() -> Settings {
    Settings {
        exporters: Vec::new(),
        service_name: "litellm".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: vec!["openinference".to_owned()],
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn build(variables: &[(&str, &str)]) -> Settings {
    SignozPreset
        .build(&environment(variables), base_settings(), false)
        .expect("the signoz preset cannot fail")
}

#[test]
fn preset_appends_the_configured_exporter_and_the_genai_mapper() {
    let settings = build(&[
        (
            SIGNOZ_INGESTION_ENDPOINT_ENV,
            "https://collector.example:4318",
        ),
        ("SIGNOZ_INGESTION_KEY", "key-1"),
    ]);
    assert_eq!(
        settings.exporters,
        [ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint: Some("https://collector.example:4318".to_owned()),
            traces_endpoint: None,
            headers: Some("signoz-ingestion-key=key-1".to_owned()),
            owner: Some(ExporterOwner::Signoz),
            options: None,
            use_simple_processor: None,
            requires_headers: true,
        }]
    );
    assert_eq!(settings.mapper_names, ["openinference", "genai"]);
}

#[test]
fn preset_passes_the_endpoint_through_without_a_traces_path() {
    let settings = build(&[(SIGNOZ_INGESTION_ENDPOINT_ENV, "collector.example:4317")]);
    assert_eq!(
        settings.exporters[0].endpoint.as_deref(),
        Some("collector.example:4317")
    );
}

#[rstest]
#[case::missing(&[], None)]
#[case::empty(&[(SIGNOZ_INGESTION_ENDPOINT_ENV, "")], Some(""))]
fn preset_keeps_the_endpoint_value(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = build(variables);
    assert_eq!(settings.exporters[0].endpoint.as_deref(), expected);
}

#[rstest]
#[case::key_and_endpoint(
    &[
        (SIGNOZ_INGESTION_ENDPOINT_ENV, "https://collector.example"),
        ("SIGNOZ_INGESTION_KEY", "key-1")
    ],
    Some("signoz-ingestion-key=key-1"),
    true
)]
#[case::key_only(&[("SIGNOZ_INGESTION_KEY", "key-1")], Some("signoz-ingestion-key=key-1"), true)]
#[case::endpoint_only(&[(SIGNOZ_INGESTION_ENDPOINT_ENV, "https://collector.example")], None, false)]
#[case::empty_key(
    &[("SIGNOZ_INGESTION_KEY", "")],
    None,
    false
)]
#[case::nothing(&[], None, false)]
fn preset_gates_the_headers_on_a_non_empty_key(
    #[case] variables: &[(&str, &str)],
    #[case] headers: Option<&str>,
    #[case] requires_headers: bool,
) {
    let settings = build(variables);
    assert_eq!(settings.exporters[0].headers.as_deref(), headers);
    assert_eq!(settings.exporters[0].requires_headers, requires_headers);
}

#[rstest]
#[case::missing(&[], None)]
#[case::empty(&[("signoz_ingestion_endpoint", "")], None)]
#[case::bare_host(&[("signoz_ingestion_endpoint", "collector.example:4318")], None)]
#[case::grpc_scheme(
    &[
        ("signoz_ingestion_endpoint", "grpc://collector.example:4317"),
        ("signoz_ingestion_key", "key-1")
    ],
    None
)]
#[case::keyless(
    &[("signoz_ingestion_endpoint", "https://collector.example/v1/traces")],
    None
)]
#[case::allowlisted(
    &[
        ("signoz_ingestion_endpoint", "https://collector.example/v1/traces"),
        ("signoz_ingestion_key", "key-1")
    ],
    Some("https://collector.example/v1/traces")
)]
fn dynamic_endpoint_requires_scheme_key_and_allowlist(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let hosts = allowlist(&["collector.example"]);
    assert_eq!(
        signoz_dynamic_endpoint(&params(variables), &hosts).as_deref(),
        expected
    );
}

#[test]
fn dynamic_endpoint_rejects_a_host_outside_the_allowlist() {
    let given = params(&[
        (
            "signoz_ingestion_endpoint",
            "https://collector.example/v1/traces",
        ),
        ("signoz_ingestion_key", "key-1"),
    ]);
    assert_eq!(
        signoz_dynamic_endpoint(&given, &allowlist(&["other.example"])),
        None
    );
    assert_eq!(signoz_dynamic_endpoint(&given, &allowlist(&[])), None);
}

#[test]
fn dynamic_endpoint_accepts_a_scheme_and_port_entry() {
    let given = params(&[
        (
            "signoz_ingestion_endpoint",
            "http://collector.example:4318/v1/traces",
        ),
        ("signoz_ingestion_key", "key-1"),
    ]);
    let hosts = allowlist(&["http://collector.example:4318"]);
    assert_eq!(
        signoz_dynamic_endpoint(&given, &hosts).as_deref(),
        Some("http://collector.example:4318/v1/traces")
    );
}

#[rstest]
#[case::key_with_empty_endpoint(
    &[
        ("signoz_ingestion_endpoint", ""),
        ("signoz_ingestion_key", "key-1")
    ],
    Some("key-1")
)]
#[case::key_with_allowlisted_endpoint(
    &[
        ("signoz_ingestion_endpoint", "https://collector.example/v1/traces"),
        ("signoz_ingestion_key", "key-1")
    ],
    Some("key-1")
)]
#[case::key_without_endpoint(&[("signoz_ingestion_key", "key-1")], Some("key-1"))]
#[case::key_with_bare_host(
    &[
        ("signoz_ingestion_endpoint", "collector.example:4318"),
        ("signoz_ingestion_key", "key-1")
    ],
    None
)]
#[case::key_with_unlisted_host(
    &[
        ("signoz_ingestion_endpoint", "https://elsewhere.example/v1/traces"),
        ("signoz_ingestion_key", "key-1")
    ],
    None
)]
#[case::no_key(
    &[("signoz_ingestion_endpoint", "https://collector.example/v1/traces")],
    None
)]
fn dynamic_headers_drop_the_key_when_the_tenant_endpoint_is_unusable(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let hosts = allowlist(&["collector.example"]);
    let headers = signoz_dynamic_headers(&params(variables), &hosts);
    assert_eq!(
        headers.get("signoz-ingestion-key").map(String::as_str),
        expected
    );
    assert_eq!(headers.len(), usize::from(expected.is_some()));
}
