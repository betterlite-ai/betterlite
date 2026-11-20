#![cfg(test)]

use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build};

const KEYS: [(&str, &str); 2] = [
    ("LANGFUSE_PUBLIC_KEY", "pk-lf-test"),
    ("LANGFUSE_SECRET_KEY", "sk-lf-test"),
];

#[test]
fn without_the_switch_langfuse_otel_builds_the_legacy_logger() {
    let inputs = catalog_inputs("", &KEYS);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[test]
fn with_the_switch_on_langfuse_otel_builds_through_the_runtime() {
    let fixture = fixture();
    fixture.environment.set(KEYS[0].0, KEYS[0].1);
    fixture.environment.set(KEYS[1].0, KEYS[1].1);
    let inputs = catalog_inputs("", &[KEYS[0], KEYS[1], ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn with_the_switch_on_and_no_runtime_langfuse_otel_is_unavailable() {
    let inputs = catalog_inputs("", &[KEYS[0], KEYS[1], ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == CALLBACK_NAME));
}
