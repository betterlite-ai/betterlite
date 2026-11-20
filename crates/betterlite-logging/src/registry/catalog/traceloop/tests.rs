#![cfg(test)]

use crate::callback_logger::CallbackLogger as _;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build};

#[test]
fn without_the_switch_traceloop_builds_the_legacy_logger() {
    let inputs = catalog_inputs("", &[]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[test]
fn with_the_switch_on_traceloop_has_no_preset_and_stays_legacy() {
    let fixture = fixture();
    let inputs = catalog_inputs("", &[("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert!(fixture.constructor.records().is_empty());
}

#[test]
fn with_the_switch_on_and_no_runtime_traceloop_is_unavailable() {
    let inputs = catalog_inputs("", &[("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == CALLBACK_NAME));
}
