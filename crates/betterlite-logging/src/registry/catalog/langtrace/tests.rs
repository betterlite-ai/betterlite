#![cfg(test)]

use crate::callback_logger::CallbackLogger;
use crate::otel::legacy_policy::config::Exporter;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build, legacy_config};

const KEY: (&str, &str) = ("LANGTRACE_API_KEY", "langtrace-key");

#[test]
fn the_legacy_config_names_the_host_endpoint_and_the_key_header() {
    let inputs = catalog_inputs(
        "",
        &[KEY, ("LANGTRACE_API_HOST", "https://langtrace.example")],
    );
    let shared = SharedOtel::new(&inputs);
    let config = legacy_config(&inputs, &shared).expect("the key is set");
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://langtrace.example/api/trace")
    );
    assert_eq!(config.headers.as_deref(), Some("x-api-key=langtrace-key"));
}

#[test]
fn without_a_host_the_default_langtrace_endpoint_is_used() {
    let inputs = catalog_inputs("", &[KEY]);
    let shared = SharedOtel::new(&inputs);
    let config = legacy_config(&inputs, &shared).expect("the key is set");
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://app.langtrace.ai/api/trace")
    );
}

#[test]
fn a_missing_api_key_gives_an_init_error_before_the_switch() {
    let inputs = catalog_inputs("", &[("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the key is required");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("LANGTRACE_API_KEY")
    ));
}

#[test]
fn without_the_switch_langtrace_builds_the_legacy_logger() {
    let inputs = catalog_inputs("", &[KEY]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[test]
fn with_the_switch_on_langtrace_builds_through_the_runtime() {
    let fixture = fixture();
    let inputs = catalog_inputs("", &[KEY, ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn with_the_switch_on_and_no_runtime_langtrace_is_unavailable() {
    let inputs = catalog_inputs("", &[KEY, ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == CALLBACK_NAME));
}
