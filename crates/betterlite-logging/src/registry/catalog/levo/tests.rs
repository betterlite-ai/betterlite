#![cfg(test)]

use crate::otel::legacy_policy::config::Exporter;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build, legacy_config};

#[test]
fn the_legacy_config_carries_the_collector_url_and_the_three_part_header() {
    let inputs = catalog_inputs("", &LEVO);
    let shared = SharedOtel::new(&inputs);
    let config = legacy_config(&inputs, &shared).expect("the credentials are set");
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://levo.example/v1/traces")
    );
    assert_eq!(
        config.headers.as_deref(),
        Some(
            "Authorization=Bearer levo-key,x-levo-organization-id=levo-org,x-levo-workspace-id=levo-workspace"
        )
    );
}

const LEVO: [(&str, &str); 4] = [
    ("LEVOAI_API_KEY", "levo-key"),
    ("LEVOAI_ORG_ID", "levo-org"),
    ("LEVOAI_WORKSPACE_ID", "levo-workspace"),
    ("LEVOAI_COLLECTOR_URL", "https://levo.example/v1/traces"),
];

#[test]
fn missing_credentials_give_an_init_error() {
    let inputs = catalog_inputs("", &[]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the credentials are required");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("LEVOAI_API_KEY")
    ));
}

#[test]
fn without_the_switch_levo_builds_the_legacy_logger() {
    let inputs = catalog_inputs("", &LEVO);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[test]
fn with_the_switch_on_levo_builds_through_the_runtime() {
    let fixture = fixture();
    fixture.environment.set(LEVO[0].0, LEVO[0].1);
    fixture.environment.set(LEVO[1].0, LEVO[1].1);
    fixture.environment.set(LEVO[2].0, LEVO[2].1);
    fixture.environment.set(LEVO[3].0, LEVO[3].1);
    let inputs = catalog_inputs("", &[("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn with_the_switch_on_and_no_runtime_levo_is_unavailable() {
    let mut variables = LEVO.to_vec();
    variables.push(("LITELLM_OTEL_V2", "true"));
    let inputs = catalog_inputs("", &variables);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == CALLBACK_NAME));
}
