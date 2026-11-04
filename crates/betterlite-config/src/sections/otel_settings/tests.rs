#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigLoader, EnvironmentLookup};

struct Lookup(BTreeMap<String, String>);

impl EnvironmentLookup for Lookup {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

fn load(yaml: &str, env: &[(&str, &str)]) -> super::super::super::types::Config {
    let lookup = Lookup(
        env.iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    );
    ConfigLoader { lookup }
        .load(yaml)
        .expect("valid OTel settings")
}

#[test]
fn reads_callback_otel_settings_and_environment_csv() {
    let config = load(
        "callback_settings:\n  otel:\n    service_name: api\n    baggage_promoted_keys: [one, two]\n    exporters:\n      - kind: otlp_http\n        endpoint: https://collector\n",
        &[(
            "LITELLM_OTEL_BAGGAGE_PROMOTED_KEYS",
            "litellm.team.id, litellm.api_key.hash",
        )],
    );
    let settings = &config.sections.otel_settings;
    assert_eq!(settings.service_name, "api");
    assert_eq!(
        settings.baggage_promoted_keys,
        ["litellm.team.id", "litellm.api_key.hash"]
    );
    assert_eq!(settings.exporters[0].kind.as_deref(), Some("otlp_http"));
    assert_eq!(
        settings.exporters[0].endpoint.as_deref(),
        Some("https://collector")
    );
}

#[test]
fn applies_defaults_and_normalizes_capture_mode_and_mapper_names() {
    let config = load(
        "callback_settings:\n  otel:\n    capture_message_content: SPAN_AND_EVENT\n    mapper_names: [langfuse, genai, langfuse]\n    legacy_compat: true\n",
        &[],
    );
    let settings = &config.sections.otel_settings;
    assert_eq!(settings.capture_message_content, "span_and_event");
    assert_eq!(settings.service_name, "litellm");
    assert_eq!(settings.mapper_names, ["genai", "langfuse", "legacy"]);
    assert_eq!(settings.baggage_promoted_keys.len(), 6);
}

#[test]
fn folds_empty_export_list_to_single_exporter_spec() {
    let config = load(
        "callback_settings:\n  otel:\n    endpoint: https://collector\n    headers: api-key=secret\n",
        &[],
    );
    let settings = &config.sections.otel_settings;
    assert_eq!(settings.exporters.len(), 1);
    assert_eq!(settings.exporters[0].kind.as_deref(), Some("otlp_http"));
    assert_eq!(
        settings.exporters[0].headers.as_deref(),
        Some("api-key=secret")
    );
}

#[test]
fn preserves_console_exporter_kind() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: console}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].kind.as_deref(),
        Some("console")
    );
}

#[test]
fn preserves_in_memory_exporter_kind() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: in_memory}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].kind.as_deref(),
        Some("in_memory")
    );
}

#[test]
fn preserves_otlp_http_exporter_kind() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: otlp_http}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].kind.as_deref(),
        Some("otlp_http")
    );
}

#[test]
fn preserves_otlp_grpc_exporter_kind() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: otlp_grpc}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].kind.as_deref(),
        Some("otlp_grpc")
    );
}

#[test]
fn preserves_http_json_exporter_kind() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: 'http/json'}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].kind.as_deref(),
        Some("http/json")
    );
}

#[test]
fn preserves_factory_exporter_kind() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: custom_factory}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].kind.as_deref(),
        Some("custom_factory")
    );
}

#[test]
fn reads_service_exclusion_list_for_normalization() {
    let config = load(
        "callback_settings:\n  otel:\n    excluded_services: [redis, postgres, misspelled]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.excluded_services,
        ["redis", "postgres", "misspelled"]
    );
}

#[test]
fn accepts_service_exclusion_csv() {
    let config = load(
        "callback_settings:\n  otel:\n    excluded_services: redis,postgres\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.excluded_services,
        ["redis", "postgres"]
    );
}

#[test]
fn accepts_null_service_exclusions() {
    let config = load(
        "callback_settings:\n  otel:\n    excluded_services: null\n",
        &[],
    );
    assert!(config.sections.otel_settings.excluded_services.is_empty());
}

#[test]
fn drops_malformed_service_exclusion_value() {
    let config = load(
        "callback_settings:\n  otel:\n    excluded_services: false\n",
        &[],
    );
    assert!(config.sections.otel_settings.excluded_services.is_empty());
}

#[test]
fn loads_exporter_factory_fields() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters:\n      - kind: custom\n        endpoint: https://collector/base\n        traces_endpoint: https://collector/traces\n        headers: api-key=secret\n        owner: langfuse_otel\n        options: {region: eu}\n        use_simple_processor: false\n        requires_headers: true\n",
        &[],
    );
    let spec = &config.sections.otel_settings.exporters[0];
    assert_eq!(spec.endpoint.as_deref(), Some("https://collector/base"));
    assert_eq!(
        spec.traces_endpoint.as_deref(),
        Some("https://collector/traces")
    );
    assert_eq!(spec.headers.as_deref(), Some("api-key=secret"));
    assert_eq!(spec.owner.as_deref(), Some("langfuse_otel"));
    assert_eq!(
        spec.options
            .as_ref()
            .and_then(|options| options.get("region"))
            .map(String::as_str),
        Some("eu")
    );
    assert_eq!(spec.use_simple_processor, Some(false));
    assert_eq!(spec.requires_headers, Some(true));
}

#[test]
fn simple_processor_defaults_to_automatic() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: otlp_http}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].use_simple_processor,
        None
    );
}

#[test]
fn simple_processor_can_be_enabled() {
    let config = load(
        "callback_settings:\n  otel:\n    exporters: [{kind: console, use_simple_processor: true}]\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].use_simple_processor,
        Some(true)
    );
}

#[test]
fn legacy_mapper_is_not_added_when_legacy_compatibility_is_disabled() {
    let config = load(
        "callback_settings:\n  otel:\n    mapper_names: [langfuse]\n    legacy_compat: false\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.mapper_names,
        ["genai", "langfuse"]
    );
}
