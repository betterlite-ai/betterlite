#![cfg(test)]

use crate::otel::legacy_policy::config::Exporter;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build, legacy_config};

const TOKEN: (&str, &str) = ("LOGFIRE_TOKEN", "logfire-token");

#[test]
fn the_legacy_config_uses_the_default_base_url_and_the_token() {
    let inputs = catalog_inputs("", &[TOKEN]);
    let shared = SharedOtel::new(&inputs);
    let config = legacy_config(&inputs, &shared).expect("the token is set");
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://logfire-api.pydantic.dev/v1/traces")
    );
    assert_eq!(
        config.headers.as_deref(),
        Some("Authorization=logfire-token")
    );
}

#[test]
fn a_configured_base_url_replaces_the_default() {
    let inputs = catalog_inputs(
        "",
        &[TOKEN, ("LOGFIRE_BASE_URL", "https://logfire.example/")],
    );
    let shared = SharedOtel::new(&inputs);
    let config = legacy_config(&inputs, &shared).expect("the token is set");
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://logfire.example/v1/traces")
    );
}

#[test]
fn a_missing_token_gives_an_init_error_before_the_switch() {
    let inputs = catalog_inputs("", &[("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the token is required");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("LOGFIRE_TOKEN")
    ));
}

#[test]
fn without_the_switch_logfire_builds_the_legacy_opentelemetry() {
    let inputs = catalog_inputs("", &[TOKEN]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), "opentelemetry");
}

#[test]
fn with_the_switch_on_logfire_has_no_preset_and_stays_legacy() {
    let fixture = fixture();
    let inputs = catalog_inputs("", &[TOKEN, ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), "opentelemetry");
    assert!(fixture.constructor.records().is_empty());
}

#[test]
fn with_the_switch_on_and_no_runtime_logfire_is_unavailable() {
    let inputs = catalog_inputs("", &[TOKEN, ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == "logfire"));
}
