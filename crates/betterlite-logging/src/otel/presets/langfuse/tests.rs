#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;

use super::{LangfusePreset, langfuse_dynamic_headers};
use crate::langfuse_otel::error::LangfuseOtelError;
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::presets::base::{Preset as _, PresetError};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment(variables)
}

fn base_settings() -> Settings {
    Settings {
        exporters: vec![
            ExporterSpec {
                kind: "console".to_owned(),
                endpoint: None,
                traces_endpoint: None,
                headers: None,
                owner: None,
                options: None,
                use_simple_processor: None,
                requires_headers: false,
            },
            ExporterSpec {
                kind: "otlp_http".to_owned(),
                endpoint: Some("https://collector.example/v1/traces".to_owned()),
                traces_endpoint: None,
                headers: Some("x-collector=1".to_owned()),
                owner: None,
                options: None,
                use_simple_processor: None,
                requires_headers: false,
            },
        ],
        service_name: "litellm".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: vec!["genai".to_owned()],
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn build(
    variables: &[(&str, &str)],
    allow_missing_credentials: bool,
) -> Result<Settings, PresetError> {
    LangfusePreset.build(
        &environment(variables),
        base_settings(),
        allow_missing_credentials,
    )
}

fn basic(public_key: &str, secret_key: &str) -> String {
    let encoded = STANDARD.encode(format!("{public_key}:{secret_key}"));
    format!("Basic {encoded}")
}

#[test]
fn preset_appends_the_langfuse_exporter_before_the_base_exporters_are_kept() {
    let settings = build(
        &[("LANGFUSE_PUBLIC_KEY", "pk"), ("LANGFUSE_SECRET_KEY", "sk")],
        false,
    )
    .expect("the preset builds");
    assert_eq!(settings.exporters.len(), 3);
    assert_eq!(settings.exporters[0].kind, "console");
    assert_eq!(
        settings.exporters[1].endpoint.as_deref(),
        Some("https://collector.example/v1/traces")
    );
    assert_eq!(
        settings.exporters[2],
        ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint: Some("https://us.cloud.langfuse.com/api/public/otel".to_owned()),
            traces_endpoint: None,
            headers: Some(format!(
                "Authorization={},x-langfuse-ingestion-version=4",
                basic("pk", "sk")
            )),
            owner: Some(ExporterOwner::LangfuseOtel),
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        }
    );
    assert_eq!(settings.mapper_names, ["genai", "langfuse"]);
}

#[rstest]
#[case::custom_host(&[("LANGFUSE_HOST", "langfuse.example")], "https://langfuse.example/api/public/otel")]
#[case::custom_otel_host(
    &[("LANGFUSE_OTEL_HOST", "otel.example")],
    "https://otel.example/api/public/otel"
)]
#[case::otel_host_wins(
    &[("LANGFUSE_HOST", "langfuse.example"), ("LANGFUSE_OTEL_HOST", "otel.example")],
    "https://otel.example/api/public/otel"
)]
#[case::host_with_scheme(&[("LANGFUSE_HOST", "http://plain.example")], "http://plain.example/api/public/otel")]
#[case::host_with_trailing_slash(
    &[("LANGFUSE_HOST", "https://langfuse.example/")],
    "https://langfuse.example/api/public/otel"
)]
#[case::empty_otel_host_falls_through(
    &[("LANGFUSE_OTEL_HOST", ""), ("LANGFUSE_HOST", "langfuse.example")],
    "https://langfuse.example/api/public/otel"
)]
fn preset_uses_the_langfuse_host(#[case] host: &[(&str, &str)], #[case] endpoint: &str) {
    let mut variables = vec![("LANGFUSE_PUBLIC_KEY", "pk"), ("LANGFUSE_SECRET_KEY", "sk")];
    variables.extend_from_slice(host);
    let settings = build(&variables, false).expect("the preset builds");
    assert_eq!(settings.exporters[2].endpoint.as_deref(), Some(endpoint));
}

#[test]
fn preset_keeps_an_empty_service_name_and_does_not_touch_the_deployment_environment() {
    let settings = build(
        &[
            ("LANGFUSE_PUBLIC_KEY", "pk"),
            ("LANGFUSE_SECRET_KEY", "sk"),
            ("OTEL_SERVICE_NAME", "proxy"),
        ],
        false,
    )
    .expect("the preset builds");
    assert_eq!(settings.service_name, "litellm");
    assert_eq!(settings.deployment_environment, None);
}

#[rstest]
#[case::missing_public_key(&[("LANGFUSE_SECRET_KEY", "sk")])]
#[case::missing_secret_key(&[("LANGFUSE_PUBLIC_KEY", "pk")])]
#[case::empty_public_key(&[("LANGFUSE_PUBLIC_KEY", ""), ("LANGFUSE_SECRET_KEY", "sk")])]
fn preset_fails_without_both_keys(#[case] variables: &[(&str, &str)]) {
    let error = build(variables, false).expect_err("missing credentials fail the preset");
    assert!(matches!(
        error,
        PresetError::Langfuse(LangfuseOtelError::MissingCredentials)
    ));
}

#[test]
fn preset_gates_the_exporters_when_missing_credentials_are_allowed() {
    let settings = build(&[], true).expect("the gated preset builds");
    assert_eq!(
        settings.exporters,
        [
            ExporterSpec {
                kind: "otlp_http".to_owned(),
                endpoint: Some("https://collector.example/v1/traces".to_owned()),
                traces_endpoint: None,
                headers: Some("x-collector=1".to_owned()),
                owner: None,
                options: None,
                use_simple_processor: None,
                requires_headers: false,
            },
            ExporterSpec {
                kind: "console".to_owned(),
                endpoint: None,
                traces_endpoint: None,
                headers: None,
                owner: Some(ExporterOwner::LangfuseOtel),
                options: None,
                use_simple_processor: None,
                requires_headers: true,
            }
        ]
    );
    assert_eq!(settings.mapper_names, ["genai", "langfuse"]);
}

#[test]
fn preset_adds_the_langfuse_mapper_once() {
    let mut base = base_settings();
    base.mapper_names = vec!["genai".to_owned(), "langfuse".to_owned()];
    let settings = LangfusePreset
        .build(
            &environment(&[("LANGFUSE_PUBLIC_KEY", "pk"), ("LANGFUSE_SECRET_KEY", "sk")]),
            base,
            false,
        )
        .expect("the preset builds");
    assert_eq!(settings.mapper_names, ["genai", "langfuse"]);
}

#[test]
fn dynamic_headers_need_both_keys() {
    let headers = langfuse_dynamic_headers(&params(&[
        ("langfuse_public_key", "pk"),
        ("langfuse_secret_key", "sk"),
    ]));
    assert_eq!(
        headers,
        BTreeMap::from([
            ("Authorization".to_owned(), basic("pk", "sk")),
            ("x-langfuse-ingestion-version".to_owned(), "4".to_owned()),
        ])
    );
}

#[rstest]
#[case::missing_secret(&[("langfuse_public_key", "pk")])]
#[case::missing_public(&[("langfuse_secret_key", "sk")])]
#[case::empty_public(&[("langfuse_public_key", ""), ("langfuse_secret_key", "sk")])]
#[case::empty_secret(&[("langfuse_public_key", "pk"), ("langfuse_secret_key", "")])]
#[case::nothing(&[])]
fn dynamic_headers_are_empty_without_both_keys(#[case] variables: &[(&str, &str)]) {
    assert!(langfuse_dynamic_headers(&params(variables)).is_empty());
}
