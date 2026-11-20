#![cfg(test)]

use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build};

const HTTP_ENDPOINT: (&str, &str) = ("ARIZE_HTTP_ENDPOINT", "http://localhost:6006/v1/traces");

#[test]
fn without_the_switch_arize_builds_the_legacy_logger() {
    let inputs = catalog_inputs("", &[HTTP_ENDPOINT]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[test]
fn with_the_switch_on_arize_builds_through_the_runtime() {
    let fixture = fixture();
    fixture.environment.set(HTTP_ENDPOINT.0, HTTP_ENDPOINT.1);
    let inputs = catalog_inputs("", &[HTTP_ENDPOINT, ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(fixture.constructor.records().len(), 1);
    assert_eq!(in_memory.len(), 1);
}

#[test]
fn with_the_switch_on_and_no_runtime_arize_is_unavailable() {
    let inputs = catalog_inputs("", &[("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == CALLBACK_NAME));
    assert!(in_memory.is_empty());
}
