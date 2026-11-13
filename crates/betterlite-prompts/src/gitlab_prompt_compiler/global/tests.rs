#![cfg(test)]

use std::sync::PoisonError;

use betterlite_models::decisions::Json as ContractJson;

use super::{GlobalGitLabConfig, set_global_gitlab_config};
use crate::gitlab_prompt_compiler::error::InitializerError;

fn config(json_text: &str) -> ContractJson {
    serde_json::from_str(json_text).unwrap()
}

#[test]
fn the_global_config_can_be_read_after_it_is_set() {
    let global = GlobalGitLabConfig::default();
    let reader = global.clone();

    set_global_gitlab_config(
        &global,
        &config(r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#),
    )
    .unwrap();

    let (prompts_path, has_token) = reader
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map_or((None, false), |config| {
            (config.prompts_path.clone(), config.access_token.is_some())
        });

    assert_eq!(prompts_path.as_deref(), Some("prompts"));
    assert!(has_token);
}

#[test]
fn an_invalid_config_leaves_the_previous_one_in_place() {
    let global = GlobalGitLabConfig::default();
    set_global_gitlab_config(
        &global,
        &config(r#"{"project": "g/r", "access_token": "t"}"#),
    )
    .unwrap();

    let error =
        set_global_gitlab_config(&global, &ContractJson::Text("nope".to_owned())).unwrap_err();

    assert!(matches!(error, InitializerError::Config(_)));
    let has_project = {
        let stored = global.0.lock().unwrap_or_else(PoisonError::into_inner);
        stored
            .as_ref()
            .expect("the first config stays")
            .project
            .is_some()
    };
    assert!(has_project);
}

#[test]
fn a_config_that_is_null_is_rejected() {
    let global = GlobalGitLabConfig::default();

    let error = set_global_gitlab_config(&global, &ContractJson::Null).unwrap_err();

    assert!(matches!(error, InitializerError::Config(_)));
    assert!(
        global
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    );
}
