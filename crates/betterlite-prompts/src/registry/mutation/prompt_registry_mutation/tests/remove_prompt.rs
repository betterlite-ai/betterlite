#![cfg(test)]

use super::support::{RecordingCallbacks, client, prompt_spec, registry_with};
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;

#[test]
fn removes_the_prompt_from_both_maps() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    registry.remove_prompt(&registry_key, &mut callbacks);
    assert!(registry.in_memory_prompts.is_empty());
    assert!(registry.prompt_id_to_custom_prompt.is_empty());
}

#[test]
fn removes_the_registered_callback() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    registry.remove_prompt(&registry_key, &mut callbacks);
    assert_eq!(callbacks.removed(), [client(Some("greeting"))]);
}

#[test]
fn an_absent_key_is_a_no_op() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    registry.remove_prompt("missing.v1::development", &mut callbacks);
    assert!(callbacks.calls.is_empty());
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert_eq!(registry.prompt_id_to_custom_prompt.len(), 1);
    assert!(registry.in_memory_prompts.contains_key(&registry_key));
}

#[test]
fn removes_only_the_named_key() {
    let greeting = prompt_spec("greeting.v1", "development", "dotprompt");
    let other = prompt_spec("other.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&greeting, &mut callbacks);
    registry
        .initialize_prompt(&other, None, &mut callbacks)
        .expect("the second prompt registers");
    callbacks.calls.clear();
    registry.remove_prompt(&registry_key_for_prompt(&greeting), &mut callbacks);
    assert_eq!(callbacks.removed(), [client(Some("greeting"))]);
    assert_eq!(
        registry
            .in_memory_prompts
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["other.v1::development"]
    );
    assert!(
        registry
            .prompt_id_to_custom_prompt
            .contains_key("other.v1::development")
    );
}

#[test]
fn removes_only_the_named_environment() {
    let development = prompt_spec("greeting.v1", "development", "dotprompt");
    let production = prompt_spec("greeting.v1", "production", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&development, &mut callbacks);
    registry
        .initialize_prompt(&production, None, &mut callbacks)
        .expect("the production prompt registers");
    callbacks.calls.clear();
    registry.remove_prompt(&registry_key_for_prompt(&development), &mut callbacks);
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
    assert_eq!(
        registry
            .prompt_id_to_custom_prompt
            .get("greeting.v1::production"),
        Some(&client(Some("greeting")))
    );
}

#[test]
fn a_key_without_a_callback_still_removes_the_prompt() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .in_memory_prompts
        .insert(registry_key.clone(), prompt);
    registry.remove_prompt(&registry_key, &mut callbacks);
    assert!(registry.in_memory_prompts.is_empty());
    assert!(registry.prompt_id_to_custom_prompt.is_empty());
    assert!(callbacks.calls.is_empty());
}
