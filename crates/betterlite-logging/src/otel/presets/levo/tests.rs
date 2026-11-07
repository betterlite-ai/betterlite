#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::LevoPreset;
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

fn collector_environment() -> [(&'static str, &'static str); 4] {
    [
        ("LEVOAI_API_KEY", "key"),
        ("LEVOAI_ORG_ID", "org"),
        ("LEVOAI_WORKSPACE_ID", "workspace"),
        ("LEVOAI_COLLECTOR_URL", "https://levo.example/v1/traces"),
    ]
}

#[test]
fn preset_appends_one_http_exporter() {
    let settings = LevoPreset
        .build(
            &environment(&collector_environment()),
            base_settings(&["genai"]),
            false,
        )
        .expect("the levo preset builds");
    assert_eq!(settings.exporters.len(), 2);
    assert_eq!(settings.exporters[0].kind, "console");
    let exporter = &settings.exporters[1];
    assert_eq!(exporter.kind, "otlp_http");
    assert_eq!(
        exporter.endpoint.as_deref(),
        Some("https://levo.example/v1/traces")
    );
    assert_eq!(
        exporter.headers.as_deref(),
        Some("Authorization=Bearer key,x-levo-organization-id=org,x-levo-workspace-id=workspace")
    );
    assert_eq!(exporter.owner, Some(ExporterOwner::Levo));
}

#[test]
fn preset_keeps_the_mapper_names_exactly_as_given() {
    let settings = LevoPreset
        .build(
            &environment(&collector_environment()),
            base_settings(&["genai", "openinference"]),
            false,
        )
        .expect("the levo preset builds");
    assert_eq!(settings.mapper_names, ["genai", "openinference"]);
}

#[rstest]
#[case::credentials_allowed(true)]
#[case::credentials_disallowed(false)]
fn preset_returns_the_missing_variable_error_whatever_the_allow_flag_says(
    #[case] allow_missing_credentials: bool,
) {
    let error = LevoPreset
        .build(
            &environment(&[("LEVOAI_API_KEY", "key")]),
            base_settings(&["genai"]),
            allow_missing_credentials,
        )
        .expect_err("the org id is missing");
    assert_eq!(
        error.to_string(),
        "LEVOAI_ORG_ID environment variable is required for Levo integration."
    );
}
