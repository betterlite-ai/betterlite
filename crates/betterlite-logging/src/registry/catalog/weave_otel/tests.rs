#![cfg(test)]

use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::tests::support::fixture;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::catalog_inputs;

use super::{CALLBACK_NAME, build};

const WEAVE: [(&str, &str); 2] = [
    ("WANDB_API_KEY", "wandb-key"),
    ("WANDB_PROJECT_ID", "entity/project"),
];

#[test]
fn a_missing_api_key_gives_an_init_error() {
    let inputs = catalog_inputs("", &[]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the key is required");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("WANDB_API_KEY")
    ));
}

#[test]
fn a_missing_project_id_gives_an_init_error() {
    let inputs = catalog_inputs("", &[WEAVE[0]]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the project is required");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("WANDB_PROJECT_ID")
    ));
}

#[test]
fn without_the_switch_weave_otel_builds_the_legacy_logger() {
    let inputs = catalog_inputs("", &WEAVE);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, None, &shared, &mut in_memory).expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[test]
fn with_the_switch_on_weave_otel_builds_through_the_runtime() {
    let fixture = fixture();
    for (name, value) in WEAVE {
        fixture.environment.set(name, value);
    }
    let inputs = catalog_inputs("", &[WEAVE[0], WEAVE[1], ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let logger = build(&inputs, Some(&fixture.runtime), &shared, &mut in_memory)
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn with_the_switch_on_and_no_runtime_weave_otel_is_unavailable() {
    let inputs = catalog_inputs("", &[WEAVE[0], WEAVE[1], ("LITELLM_OTEL_V2", "true")]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("the runtime is required");
    assert!(matches!(error, CatalogError::Unavailable { name } if name == CALLBACK_NAME));
}
