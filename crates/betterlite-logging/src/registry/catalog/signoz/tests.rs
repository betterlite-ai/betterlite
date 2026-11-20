#![cfg(test)]

use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build};

const ENDPOINT: (&str, &str) = ("SIGNOZ_INGESTION_ENDPOINT", "http://signoz.example:4318");

#[test]
fn a_missing_ingestion_endpoint_gives_an_init_error() {
    let inputs = catalog_inputs("", &[("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the endpoint is required");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("SIGNOZ_INGESTION_ENDPOINT")
    ));
}

#[test]
fn an_empty_ingestion_endpoint_gives_an_init_error() {
    let inputs = catalog_inputs(
        "",
        &[
            ("SIGNOZ_INGESTION_ENDPOINT", ""),
            ("LITELLM_OTEL_V2", "true"),
        ],
    );
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the endpoint is required");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("SIGNOZ_INGESTION_ENDPOINT")
    ));
}

#[test]
fn without_the_switch_signoz_builds_the_legacy_logger() {
    let inputs = catalog_inputs("", &[ENDPOINT]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[test]
fn with_the_switch_on_signoz_builds_through_the_runtime() {
    let fixture = fixture();
    fixture
        .environment
        .set("SIGNOZ_INGESTION_ENDPOINT", "http://signoz.example:4318");
    let inputs = catalog_inputs("", &[ENDPOINT, ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn with_the_switch_on_and_no_runtime_signoz_is_unavailable() {
    let inputs = catalog_inputs("", &[ENDPOINT, ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == CALLBACK_NAME));
}
