#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::{WeavePreset, weave_dynamic_headers};
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

fn credentials() -> [(&'static str, &'static str); 2] {
    [
        ("WANDB_API_KEY", "abc"),
        ("WANDB_PROJECT_ID", "entity/project"),
    ]
}

#[test]
fn preset_appends_the_exporter_and_the_mappers() {
    let settings = WeavePreset
        .build(
            &environment(&credentials()),
            base_settings(&["genai"]),
            false,
        )
        .expect("the weave preset builds");
    assert_eq!(settings.exporters.len(), 2);
    assert_eq!(
        settings.exporters[1],
        ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint: Some("https://trace.wandb.ai/otel/v1/traces".to_owned()),
            traces_endpoint: None,
            headers: Some("Authorization=Basic YXBpOmFiYw==,project_id=entity/project".to_owned()),
            owner: Some(ExporterOwner::WeaveOtel),
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        }
    );
    assert_eq!(settings.mapper_names, ["genai", "openinference", "weave"]);
}

#[test]
fn preset_keeps_existing_mappers_and_adds_each_name_once() {
    let settings = WeavePreset
        .build(
            &environment(&credentials()),
            base_settings(&["genai", "weave"]),
            false,
        )
        .expect("the weave preset builds");
    assert_eq!(settings.mapper_names, ["genai", "weave", "openinference"]);
}

#[test]
fn preset_returns_the_error_without_credentials_unless_allowed() {
    let error = WeavePreset
        .build(&environment(&[]), base_settings(&["genai"]), false)
        .expect_err("the credentials are missing");
    assert_eq!(
        error.to_string(),
        "WANDB_API_KEY must be set for Weave OpenTelemetry integration."
    );
}

#[test]
fn preset_gates_the_exporters_when_credentials_are_allowed_to_be_missing() {
    let settings = WeavePreset
        .build(&environment(&[]), base_settings(&["genai"]), true)
        .expect("the gated preset builds");
    assert_eq!(
        settings.exporters,
        [ExporterSpec {
            kind: "console".to_owned(),
            endpoint: None,
            traces_endpoint: None,
            headers: None,
            owner: Some(ExporterOwner::WeaveOtel),
            options: None,
            use_simple_processor: None,
            requires_headers: true,
        }]
    );
    assert_eq!(settings.mapper_names, ["genai", "openinference", "weave"]);
}

#[test]
fn preset_gating_keeps_configured_exporters() {
    let mut base = base_settings(&["genai"]);
    base.exporters.push(ExporterSpec {
        kind: "otlp_http".to_owned(),
        endpoint: Some("https://collector.example/v1/traces".to_owned()),
        traces_endpoint: None,
        headers: None,
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers: false,
    });
    let settings = WeavePreset
        .build(&environment(&[]), base, true)
        .expect("the gated preset builds");
    assert_eq!(settings.exporters.len(), 2);
    assert_eq!(settings.exporters[0].kind, "otlp_http");
    assert_eq!(settings.exporters[1].owner, Some(ExporterOwner::WeaveOtel));
}

#[test]
fn weave_dynamic_headers_build_both_headers() {
    let params = BTreeMap::from([
        ("wandb_api_key".to_owned(), "abc".to_owned()),
        ("weave_project_id".to_owned(), "entity/project".to_owned()),
    ]);
    assert_eq!(
        weave_dynamic_headers(&params),
        BTreeMap::from([
            ("Authorization".to_owned(), "Basic YXBpOmFiYw==".to_owned()),
            ("project_id".to_owned(), "entity/project".to_owned()),
        ])
    );
}

#[test]
fn weave_dynamic_headers_skip_empty_parameters() {
    let params = BTreeMap::from([
        ("wandb_api_key".to_owned(), String::new()),
        ("weave_project_id".to_owned(), "entity/project".to_owned()),
    ]);
    assert_eq!(
        weave_dynamic_headers(&params),
        BTreeMap::from([("project_id".to_owned(), "entity/project".to_owned())])
    );
}
