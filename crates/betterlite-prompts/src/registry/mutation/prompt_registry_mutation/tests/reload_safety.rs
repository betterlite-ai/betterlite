#![cfg(test)]

use super::support::{RecordingCallbacks, client, prompt_spec, registry_with};
use crate::registry::mutation::prompt_registry_mutation::error::PromptRegistryError;
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;

#[test]
fn a_failed_build_returns_an_error_naming_the_integration() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let changed = prompt_spec("greeting.v1", "development", "langfuse");
    let error = registry
        .reload_prompt(&changed, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert!(matches!(
        error,
        PromptRegistryError::UnsupportedIntegration { integration } if integration == "langfuse"
    ));
}

#[test]
fn a_failed_build_keeps_the_stored_prompt() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut changed = prompt_spec("greeting.v1", "development", "langfuse");
    changed.version = Some(2);
    registry
        .reload_prompt(&changed, &mut callbacks)
        .expect_err("langfuse has no initializer");
    let stored = registry
        .in_memory_prompts
        .get(&registry_key_for_prompt(&prompt))
        .expect("the stored prompt survives the failed reload");
    assert_eq!(
        stored.litellm_params.prompt_integration,
        "dotprompt".to_owned()
    );
    assert_eq!(stored.version, None);
}

#[test]
fn a_failed_build_leaves_the_stale_callback_registered() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let changed = prompt_spec("greeting.v1", "development", "langfuse");
    registry
        .reload_prompt(&changed, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert_eq!(
        registry
            .prompt_id_to_custom_prompt
            .get(&registry_key_for_prompt(&prompt)),
        Some(&client(Some("greeting")))
    );
    assert!(callbacks.calls.is_empty());
}

#[test]
fn a_failed_build_removes_no_callback_from_the_logging_manager() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let changed = prompt_spec("greeting.v1", "development", "langfuse");
    registry
        .reload_prompt(&changed, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert!(callbacks.calls.is_empty());
}
