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
    assert_eq!(settings.baggage_promoted_keys, ["one", "two"]);
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
    assert_eq!(
        settings.mapper_names,
        ["genai", "langfuse", "langfuse", "legacy"]
    );
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
fn headers_only_shorthand_keeps_console_kind() {
    let config = load(
        "callback_settings:\n  otel:\n    headers: api-key=secret\n",
        &[],
    );
    assert_eq!(
        config.sections.otel_settings.exporters[0].kind.as_deref(),
        Some("console")
    );
}

#[test]
fn reads_resource_attributes_from_environment() {
    let config = load("", &[("RESOURCE_ATTRIBUTES", "deployment.tier: prod")]);
    assert_eq!(
        config.sections.otel_settings.resource_attributes,
        BTreeMap::from([("deployment.tier".to_owned(), "prod".to_owned())])
    );
}

#[test]
fn empty_resource_attributes_environment_is_ignored() {
    let config = load("", &[("RESOURCE_ATTRIBUTES", "")]);
    assert!(config.sections.otel_settings.resource_attributes.is_empty());
}

#[rstest::rstest]
#[case::console("console")]
#[case::in_memory("in_memory")]
#[case::otlp_http("otlp_http")]
#[case::otlp_grpc("otlp_grpc")]
#[case::http_json("http/json")]
#[case::factory("custom_factory")]
fn preserves_exporter_contract(
    #[case] kind: &str,
    #[values("null", "true", "false")] processor: &str,
) {
    let yaml = format!(
        "callback_settings:\n  otel:\n    exporters: [{{kind: '{kind}', use_simple_processor: {processor}}}]\n"
    );
    let config = load(&yaml, &[]);
    let spec = &config.sections.otel_settings.exporters[0];
    assert_eq!(spec.kind.as_deref(), Some(kind));
    let expected = match processor {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    assert_eq!(spec.use_simple_processor, expected);
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

#[rstest::rstest]
#[case::openinference("openinference")]
#[case::langfuse("langfuse")]
#[case::weave("weave")]
#[case::langtrace("langtrace")]
fn reads_mapper_environment(#[case] mapper: &str) {
    let config = load("", &[("mapper_names", mapper)]);
    assert_eq!(
        config.sections.otel_settings.mapper_names,
        ["genai", mapper, "legacy"]
    );
}

#[test]
fn empty_environment_uses_default() {
    let config = load("", &[("OTEL_SERVICE_NAME", "")]);
    assert_eq!(config.sections.otel_settings.service_name, "litellm");
}

#[test]
fn bare_exclusions_do_not_override_yaml_updates() {
    let env = &[("excluded_services", "redis")];
    let first = load(
        "callback_settings: {otel: {excluded_services: [postgres]}}",
        env,
    );
    let second = load("callback_settings: {otel: {excluded_services: []}}", env);
    assert_eq!(first.sections.otel_settings.excluded_services, ["postgres"]);
    assert!(second.sections.otel_settings.excluded_services.is_empty());
}

#[test]
fn missing_environment_reference_names_variable_and_path() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("callback_settings: {otel: {service_name: os.environ/OTEL_MISSING}}")
    .expect_err("missing environment");
    let message = error.to_string();
    assert!(message.contains("OTEL_MISSING"));
    assert!(message.contains("service_name"));
}

#[rstest::rstest]
#[case::absent(None, false)]
#[case::empty(Some(""), false)]
#[case::false_text(Some("false"), false)]
#[case::zero(Some("0"), false)]
#[case::off(Some("off"), false)]
#[case::true_text(Some("true"), true)]
#[case::one(Some("1"), true)]
fn feature_flag_is_cached(#[case] value: Option<&str>, #[case] expected: bool) {
    let mut flag = super::FeatureFlag::default();
    let mut env = BTreeMap::new();
    if let Some(text) = value {
        env.insert("LITELLM_OTEL_V2".to_owned(), text.to_owned());
    }
    assert_eq!(flag.enabled(&env).expect("flag"), expected);
    env.insert("LITELLM_OTEL_V2".to_owned(), (!expected).to_string());
    assert_eq!(flag.enabled(&env).expect("cached flag"), expected);
}

#[test]
fn defaults_preserve_unconfigured_exporter_and_baggage() {
    let config = load("", &[]);
    let settings = &config.sections.otel_settings;
    assert_eq!(settings.exporters, [super::ExporterSpec::default()]);
    assert_eq!(
        settings.baggage_promoted_keys,
        [
            "litellm.team.id",
            "litellm.team.alias",
            "litellm.team.metadata",
            "litellm.api_key.hash",
            "gen_ai.request.model",
            "litellm.provider.model"
        ]
    );
    assert_eq!(
        settings.baggage_metadata_keys,
        [
            "user_api_key_org_id",
            "user_api_key_user_id",
            "user_api_key_alias",
            "user_api_key_end_user_id",
            "requester_ip_address"
        ]
    );
    assert!(settings.baggage_team_metadata_keys.is_empty());
    assert_eq!(settings.capture_message_content, "no_content");
    assert_eq!(settings.langfuse_span_scope, "full");
    assert!(!settings.enable_events);
    assert!(!settings.enable_metrics);
    assert!(settings.legacy_compat);
}

#[test]
fn yaml_aliases_and_csv_preserve_punctuation() {
    let config = load(
        "callback_settings:\n  otel:\n    OTEL_SERVICE_NAME: named\n    LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS: 'region:zone, team.id'\n",
        &[],
    );
    assert_eq!(config.sections.otel_settings.service_name, "named");
    assert_eq!(
        config.sections.otel_settings.baggage_team_metadata_keys,
        ["region:zone", "team.id"]
    );
}
