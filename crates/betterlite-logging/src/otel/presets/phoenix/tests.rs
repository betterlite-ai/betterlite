#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{PhoenixPreset, phoenix_project_headers};
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
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn build(variables: &[(&str, &str)], mapper_names: &[&str], allow: bool) -> Settings {
    PhoenixPreset
        .build(&environment(variables), base_settings(mapper_names), allow)
        .expect("the phoenix preset builds")
}

#[test]
fn preset_appends_the_exporter_and_the_mapper() {
    let settings = build(&[("PHOENIX_API_KEY", "key")], &["genai"], false);
    assert_eq!(settings.exporters.len(), 2);
    let exporter = &settings.exporters[1];
    assert_eq!(exporter.kind, "otlp_http");
    assert_eq!(
        exporter.endpoint.as_deref(),
        Some("http://localhost:6006/v1/traces")
    );
    assert_eq!(
        exporter.headers.as_deref(),
        Some("Authorization=Bearer key")
    );
    assert_eq!(exporter.owner, Some(ExporterOwner::ArizePhoenix));
    assert_eq!(settings.mapper_names, ["genai", "openinference"]);
}

#[rstest]
#[case::project_name(&[("PHOENIX_PROJECT_NAME", "named")], "named")]
#[case::collector_alias(&[("PHOENIX_COLLECTOR_PROJECT_NAME", "alias")], "alias")]
#[case::name_beats_alias(
    &[
        ("PHOENIX_PROJECT_NAME", "named"),
        ("PHOENIX_COLLECTOR_PROJECT_NAME", "alias")
    ],
    "named"
)]
#[case::default(&[], "default")]
fn preset_sets_the_project_resource_attribute(
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings = build(variables, &["genai"], false);
    assert_eq!(
        settings
            .resource_attributes
            .get("openinference.project.name")
            .map(String::as_str),
        Some(expected)
    );
}

#[rstest]
#[case::credentials_allowed(true)]
#[case::credentials_disallowed(false)]
fn preset_returns_the_cloud_error_whatever_the_allow_flag_says(
    #[case] allow_missing_credentials: bool,
) {
    let error = PhoenixPreset
        .build(
            &environment(&[(
                "PHOENIX_COLLECTOR_ENDPOINT",
                "https://app.phoenix.arize.com",
            )]),
            base_settings(&["genai"]),
            allow_missing_credentials,
        )
        .expect_err("the cloud needs a key");
    assert_eq!(
        error.to_string(),
        "PHOENIX_API_KEY must be set when using Phoenix Cloud (app.phoenix.arize.com)."
    );
}

#[test]
fn phoenix_project_headers_take_the_override_first() {
    let metadata = BTreeMap::from([
        (
            "phoenix_project_name_override".to_owned(),
            "override".to_owned(),
        ),
        ("phoenix_project_name".to_owned(), "named".to_owned()),
    ]);
    assert_eq!(
        phoenix_project_headers(&metadata),
        BTreeMap::from([("x-project-name".to_owned(), "override".to_owned())])
    );
}

#[test]
fn phoenix_project_headers_fall_back_to_the_project_name() {
    let metadata = BTreeMap::from([("phoenix_project_name".to_owned(), " named ".to_owned())]);
    assert_eq!(
        phoenix_project_headers(&metadata),
        BTreeMap::from([("x-project-name".to_owned(), "named".to_owned())])
    );
}

#[test]
fn phoenix_project_headers_skip_blank_values() {
    let metadata = BTreeMap::from([
        ("phoenix_project_name_override".to_owned(), "  ".to_owned()),
        ("phoenix_project_name".to_owned(), "named".to_owned()),
    ]);
    assert_eq!(
        phoenix_project_headers(&metadata),
        BTreeMap::from([("x-project-name".to_owned(), "named".to_owned())])
    );
}

#[test]
fn phoenix_project_headers_are_empty_without_a_project() {
    assert_eq!(phoenix_project_headers(&BTreeMap::new()), BTreeMap::new());
    let metadata = BTreeMap::from([("phoenix_project_name".to_owned(), String::new())]);
    assert_eq!(phoenix_project_headers(&metadata), BTreeMap::new());
}
