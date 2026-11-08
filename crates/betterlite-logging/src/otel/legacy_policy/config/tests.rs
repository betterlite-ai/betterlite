#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{
    DEFAULT_ENVIRONMENT, DEFAULT_SERVICE_NAME, Exporter, MetadataKeys, OpenTelemetryConfig,
    Signals, normalize_metadata_keys,
};
use crate::otel::vocabulary::semconv::OTELSemconvCategory;

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn base_config() -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter: Exporter::Console,
        endpoint: None,
        headers: None,
        signals: Signals::default(),
        service_name: String::new(),
        deployment_environment: String::new(),
        model_id: String::new(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

#[test]
fn from_env_defaults_match_litellm() {
    let config = OpenTelemetryConfig::from_env(&BTreeMap::new());
    assert_eq!(config.exporter, Exporter::Console);
    assert_eq!(config.endpoint, None);
    assert_eq!(config.headers, None);
    assert_eq!(config.service_name, DEFAULT_SERVICE_NAME);
    assert_eq!(config.deployment_environment, DEFAULT_ENVIRONMENT);
    assert_eq!(config.model_id, DEFAULT_SERVICE_NAME);
}

#[test]
fn from_env_defaults_leave_switches_and_extras_off() {
    let config = OpenTelemetryConfig::from_env(&BTreeMap::new());
    assert!(!config.signals.enable_metrics);
    assert!(!config.signals.enable_events);
    assert!(!config.ignore_context_propagation);
    assert!(!config.skip_set_global);
    assert_eq!(config.capture_message_content, None);
    assert!(config.semconv_stability_opt_in.is_empty());
    assert!(config.baggage_team_metadata_keys.is_empty());
    assert!(config.baggage_metadata_keys.is_empty());
    assert!(config.attributes.is_none());
}

#[test]
fn from_env_reads_every_documented_variable() {
    let environment = env(&[
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf"),
        ("OTEL_EXPORTER", "otlp_http"),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://collector/v1/traces"),
        ("OTEL_ENDPOINT", "https://ignored/v1/traces"),
        ("OTEL_EXPORTER_OTLP_HEADERS", "x-team=one"),
        ("OTEL_HEADERS", "x-ignored=two"),
        ("LITELLM_OTEL_INTEGRATION_ENABLE_METRICS", "TRUE"),
        ("LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS", "True"),
        ("OTEL_SERVICE_NAME", "gateway"),
        ("OTEL_ENVIRONMENT_NAME", "staging"),
        ("OTEL_MODEL_ID", "gateway-model"),
    ]);
    let config = OpenTelemetryConfig::from_env(&environment);
    assert_eq!(config.exporter, Exporter::Named("http/protobuf".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://collector/v1/traces")
    );
    assert_eq!(config.headers.as_deref(), Some("x-team=one"));
    assert!(config.signals.enable_metrics);
    assert!(config.signals.enable_events);
    assert_eq!(config.service_name, "gateway");
    assert_eq!(config.deployment_environment, "staging");
    assert_eq!(config.model_id, "gateway-model");
}

#[rstest]
#[case::upper_true("TRUE", true)]
#[case::mixed_true("True", true)]
#[case::one_is_not_true("1", false)]
#[case::yes_is_not_true("yes", false)]
#[case::padded_true(" true", false)]
fn from_env_switches_read_only_true(#[case] text: &str, #[case] expected: bool) {
    let environment = env(&[
        ("LITELLM_OTEL_INTEGRATION_ENABLE_METRICS", text),
        ("LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS", text),
    ]);
    let config = OpenTelemetryConfig::from_env(&environment);
    assert_eq!(config.signals.enable_metrics, expected);
    assert_eq!(config.signals.enable_events, expected);
}

#[test]
fn from_env_in_memory_keeps_every_other_field_at_default() {
    let environment = env(&[
        ("OTEL_EXPORTER", "in_memory"),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://collector/v1/traces"),
        ("OTEL_EXPORTER_OTLP_HEADERS", "x-team=one"),
        ("LITELLM_OTEL_INTEGRATION_ENABLE_METRICS", "true"),
        ("LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS", "true"),
    ]);
    let config = OpenTelemetryConfig::from_env(&environment);
    assert_eq!(config.exporter, Exporter::InMemory);
    assert_eq!(config.endpoint, None);
    assert_eq!(config.headers, None);
    assert!(!config.signals.enable_metrics);
    assert!(!config.signals.enable_events);
    assert!(config.service_name.is_empty());
    assert!(config.model_id.is_empty());
}

#[test]
fn normalize_turns_console_with_endpoint_into_otlp_http() {
    let mut config = base_config();
    config.endpoint = Some("https://collector/v1/traces".to_owned());
    config.normalize(&BTreeMap::new());
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://collector/v1/traces")
    );
}

#[test]
fn normalize_keeps_a_named_exporter() {
    let mut config = base_config();
    config.exporter = Exporter::Named("otlp_grpc".to_owned());
    config.endpoint = Some("https://collector".to_owned());
    config.normalize(&BTreeMap::new());
    assert_eq!(config.exporter, Exporter::Named("otlp_grpc".to_owned()));
}

#[test]
fn normalize_fills_empty_resource_fields_from_env() {
    let environment = env(&[
        ("OTEL_SERVICE_NAME", "svc"),
        ("OTEL_ENVIRONMENT_NAME", "staging"),
        ("OTEL_MODEL_ID", "served-model"),
    ]);
    let mut config = base_config();
    config.normalize(&environment);
    assert_eq!(config.service_name, "svc");
    assert_eq!(config.deployment_environment, "staging");
    assert_eq!(config.model_id, "served-model");
}

#[test]
fn normalize_defaults_the_model_id_to_the_service_name() {
    let environment = env(&[("OTEL_SERVICE_NAME", "svc")]);
    let mut config = base_config();
    config.normalize(&environment);
    assert_eq!(config.model_id, "svc");
}

#[test]
fn normalize_leaves_explicit_resource_fields_alone() {
    let environment = env(&[
        ("OTEL_SERVICE_NAME", "env-service"),
        ("OTEL_ENVIRONMENT_NAME", "env-environment"),
        ("OTEL_MODEL_ID", "env-model"),
    ]);
    let mut config = base_config();
    config.service_name = "given-service".to_owned();
    config.deployment_environment = "given-environment".to_owned();
    config.model_id = "given-model".to_owned();
    config.normalize(&environment);
    assert_eq!(config.service_name, "given-service");
    assert_eq!(config.deployment_environment, "given-environment");
    assert_eq!(config.model_id, "given-model");
}

#[rstest]
#[case::lower_true("true", true)]
#[case::upper_true(" TRUE ", true)]
#[case::lower_false("false", false)]
#[case::one_is_unknown("1", false)]
#[case::empty("", false)]
fn normalize_reads_the_ignore_context_flag(#[case] text: &str, #[case] expected: bool) {
    let environment = env(&[("OTEL_IGNORE_CONTEXT_PROPAGATION", text)]);
    let mut config = base_config();
    config.normalize(&environment);
    assert_eq!(config.ignore_context_propagation, expected);
}

#[test]
fn normalize_keeps_an_explicit_ignore_context_flag() {
    let environment = env(&[("OTEL_IGNORE_CONTEXT_PROPAGATION", "false")]);
    let mut config = base_config();
    config.ignore_context_propagation = true;
    config.normalize(&environment);
    assert!(config.ignore_context_propagation);
}

#[test]
fn normalize_unions_the_semconv_opt_in() {
    let environment = env(&[(
        "OTEL_SEMCONV_STABILITY_OPT_IN",
        "http, gen_ai_latest_experimental",
    )]);
    let mut config = base_config();
    config.normalize(&environment);
    assert_eq!(
        config.semconv_stability_opt_in,
        BTreeSet::from([OTELSemconvCategory::GenAiLatestExperimental])
    );
}

#[test]
fn normalize_falls_back_to_the_baggage_env_only_when_empty() {
    let environment = env(&[
        (
            "LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS",
            " region , ,owner ",
        ),
        ("LITELLM_OTEL_BAGGAGE_METADATA_KEYS", "team,trace_id"),
    ]);
    let mut config = base_config();
    config.baggage_team_metadata_keys = vec![" kept ".to_owned()];
    config.normalize(&environment);
    assert_eq!(config.baggage_team_metadata_keys, vec!["kept".to_owned()]);
    assert_eq!(
        config.baggage_metadata_keys,
        vec!["team".to_owned(), "trace_id".to_owned()]
    );
}

#[test]
fn normalize_keeps_empty_baggage_keys_without_env() {
    let mut config = base_config();
    config.normalize(&BTreeMap::new());
    assert!(config.baggage_team_metadata_keys.is_empty());
    assert!(config.baggage_metadata_keys.is_empty());
}

#[rstest]
#[case::csv(MetadataKeys::Text(" one, two ,, three "), vec!["one", "two", "three"])]
#[case::empty_csv(MetadataKeys::Text(" , , "), Vec::new())]
#[case::list_entries(
    MetadataKeys::List(&[" one ".to_owned(), String::new(), "two".to_owned()]),
    vec!["one", "two"]
)]
fn normalize_metadata_keys_strips_entries(
    #[case] input: MetadataKeys<'_>,
    #[case] expected: Vec<&str>,
) {
    assert_eq!(normalize_metadata_keys(input), expected);
}
