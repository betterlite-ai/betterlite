#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::LangtracePreset;
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterSpec, Settings};
use crate::otel::presets::base::Preset as _;

fn base_settings(mapper_names: &[&str]) -> Settings {
    Settings {
        exporters: vec![ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint: Some("https://collector.example/v1/traces".to_owned()),
            traces_endpoint: None,
            headers: Some("x=1".to_owned()),
            owner: None,
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        }],
        service_name: "litellm".to_owned(),
        deployment_environment: Some("production".to_owned()),
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

#[test]
fn preset_only_appends_the_langtrace_mapper() {
    let base = base_settings(&["genai", "openinference"]);
    let settings = LangtracePreset
        .build(&BTreeMap::new(), base.clone(), false)
        .expect("the langtrace preset builds");
    assert_eq!(
        settings.mapper_names,
        ["genai", "openinference", "langtrace"]
    );
    assert_eq!(settings.exporters, base.exporters);
    assert_eq!(settings.resource_attributes, base.resource_attributes);
}

#[test]
fn preset_keeps_the_mapper_names_when_langtrace_is_present() {
    let settings = LangtracePreset
        .build(
            &BTreeMap::new(),
            base_settings(&["genai", "langtrace"]),
            false,
        )
        .expect("the langtrace preset builds");
    assert_eq!(settings.mapper_names, ["genai", "langtrace"]);
}
