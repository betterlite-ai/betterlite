#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{ArizePreset, arize_dynamic_headers};
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::presets::base::Preset as _;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn base_settings(mapper_names: &[&str]) -> Settings {
    Settings {
        exporters: vec![ExporterSpec {
            kind: "console".to_owned(),
            endpoint: None,
            traces_endpoint: None,
            headers: None,
            owner: None,
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        }],
        service_name: "litellm".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: mapper_names.iter().map(|name| (*name).to_owned()).collect(),
        resource_attributes: BTreeMap::from([("service.name".to_owned(), "proxy".to_owned())]),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn build(variables: &[(&str, &str)], mapper_names: &[&str]) -> Settings {
    ArizePreset
        .build(&environment(variables), base_settings(mapper_names), false)
        .expect("the arize preset builds")
}

#[test]
fn preset_appends_the_exporter_and_the_mapper() {
    let settings = build(&[("ARIZE_SPACE_ID", "space")], &["genai"]);
    assert_eq!(settings.exporters.len(), 2);
    assert_eq!(settings.exporters[0].kind, "console");
    let exporter = &settings.exporters[1];
    assert_eq!(exporter.kind, "otlp_grpc");
    assert_eq!(
        exporter.endpoint.as_deref(),
        Some("https://otlp.arize.com/v1")
    );
    assert_eq!(exporter.owner, Some(ExporterOwner::ArizeAx));
    assert_eq!(settings.mapper_names, ["genai", "openinference"]);
}

#[rstest]
#[case::space_id_beats_space_key(
    &[("ARIZE_SPACE_ID", "id"), ("ARIZE_SPACE_KEY", "key")],
    Some("space_id=id")
)]
#[case::space_key_is_the_fallback(&[("ARIZE_SPACE_KEY", "key")], Some("space_id=key"))]
#[case::api_key_with_space(
    &[("ARIZE_SPACE_ID", "id"), ("ARIZE_API_KEY", "api")],
    Some("space_id=id,api_key=api")
)]
#[case::api_key_alone(&[("ARIZE_API_KEY", "api")], Some("api_key=api"))]
#[case::otlp_headers_fallback(
    &[("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "x=a,b=c")],
    Some("x=a,b=c")
)]
#[case::no_credentials(&[], None)]
fn preset_builds_the_headers(#[case] variables: &[(&str, &str)], #[case] expected: Option<&str>) {
    let settings = build(variables, &["genai"]);
    assert_eq!(settings.exporters[1].headers.as_deref(), expected);
}

#[test]
fn preset_sets_model_id_only_with_a_project_name() {
    let settings = build(&[("ARIZE_PROJECT_NAME", "project")], &["genai"]);
    assert_eq!(
        settings
            .resource_attributes
            .get("model_id")
            .map(String::as_str),
        Some("project")
    );
    assert_eq!(
        settings
            .resource_attributes
            .get("service.name")
            .map(String::as_str),
        Some("proxy")
    );
    let without = build(&[], &["genai"]);
    assert_eq!(without.resource_attributes.get("model_id"), None);
}

#[test]
fn preset_keeps_existing_mappers_and_adds_openinference_once() {
    let settings = build(&[], &["genai", "openinference", "weave"]);
    assert_eq!(settings.mapper_names, ["genai", "openinference", "weave"]);
}

#[test]
fn arize_dynamic_headers_prefer_the_space_key() {
    let params = BTreeMap::from([
        ("arize_space_key".to_owned(), "key".to_owned()),
        ("arize_space_id".to_owned(), "id".to_owned()),
        ("arize_api_key".to_owned(), "api".to_owned()),
    ]);
    assert_eq!(
        arize_dynamic_headers(&params),
        BTreeMap::from([
            ("arize-space-id".to_owned(), "key".to_owned()),
            ("api_key".to_owned(), "api".to_owned()),
        ])
    );
}

#[test]
fn arize_dynamic_headers_fall_back_to_the_space_id() {
    let params = BTreeMap::from([("arize_space_id".to_owned(), "id".to_owned())]);
    assert_eq!(
        arize_dynamic_headers(&params),
        BTreeMap::from([("arize-space-id".to_owned(), "id".to_owned())])
    );
}

#[test]
fn arize_dynamic_headers_skip_empty_parameters() {
    let params = BTreeMap::from([
        ("arize_space_key".to_owned(), String::new()),
        ("arize_space_id".to_owned(), "id".to_owned()),
        ("arize_api_key".to_owned(), String::new()),
    ]);
    assert_eq!(
        arize_dynamic_headers(&params),
        BTreeMap::from([("arize-space-id".to_owned(), "id".to_owned())])
    );
}
