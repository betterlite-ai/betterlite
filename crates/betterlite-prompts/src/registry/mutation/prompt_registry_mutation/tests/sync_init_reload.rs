#![cfg(test)]

use super::support::{CallbackCall, RecordingCallbacks, client, prompt_spec, registry_with};
use crate::registry::mutation::prompt_registry_mutation::error::PromptRegistryError;
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;

#[test]
fn a_missing_prompt_is_initialized() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    let parsed = registry
        .sync_prompt_from_db(&prompt, &mut callbacks)
        .expect("the unseen row initializes");
    assert_eq!(parsed.prompt_id, "greeting.v1");
    assert_eq!(
        callbacks.calls,
        [CallbackCall::Added(client(Some("greeting")))]
    );
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert_eq!(registry.prompt_id_to_custom_prompt.len(), 1);
}

#[test]
fn an_initialized_prompt_is_stored_under_its_environment() {
    let prompt = prompt_spec("greeting.v1", "production", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .sync_prompt_from_db(&prompt, &mut callbacks)
        .expect("the unseen row initializes");
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v1::production")
    );
    assert!(
        registry
            .prompt_id_to_custom_prompt
            .contains_key("greeting.v1::production")
    );
}

#[test]
fn a_differing_row_reloads_the_stored_prompt() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    row.litellm_params.prompt_integration = "bitbucket".to_owned();
    row.litellm_params.prompt_id = Some("repo".to_owned());
    registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect("the differing row reloads");
    assert_eq!(
        callbacks.calls,
        [
            CallbackCall::Removed(client(Some("greeting"))),
            CallbackCall::Added(client(Some("repo"))),
        ]
    );
    assert_eq!(
        registry
            .in_memory_prompts
            .get(&registry_key)
            .map(|stored| stored.litellm_params.prompt_integration.clone()),
        Some("bitbucket".to_owned())
    );
}

#[test]
fn a_row_that_differs_only_in_prompt_info_reloads() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    row.prompt_info.prompt_type = "db".to_owned();
    registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect("the differing row reloads");
    assert!(matches!(
        callbacks.calls.as_slice(),
        [CallbackCall::Removed(_), CallbackCall::Added(_)]
    ));
    assert_eq!(
        registry
            .in_memory_prompts
            .get(&registry_key)
            .map(|stored| stored.prompt_info.prompt_type.clone()),
        Some("db".to_owned())
    );
}

#[test]
fn an_unsupported_integration_on_a_new_row_registers_nothing() {
    let prompt = prompt_spec("greeting.v1", "development", "langfuse");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    let error = registry
        .sync_prompt_from_db(&prompt, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert!(matches!(
        error,
        PromptRegistryError::UnsupportedIntegration { integration } if integration == "langfuse"
    ));
    assert!(registry.in_memory_prompts.is_empty());
    assert!(registry.prompt_id_to_custom_prompt.is_empty());
    assert!(callbacks.calls.is_empty());
}

#[test]
fn a_failed_reload_keeps_the_stored_prompt() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    row.litellm_params.prompt_integration = "langfuse".to_owned();
    registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert_eq!(
        registry
            .in_memory_prompts
            .get(&registry_key)
            .map(|stored| stored.litellm_params.prompt_integration.clone()),
        Some("dotprompt".to_owned())
    );
    assert!(callbacks.calls.is_empty());
}
