#![cfg(test)]

use super::support::{CallbackCall, RecordingCallbacks, client, prompt_spec, registry_with};
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;

fn registered(
    library: &mut InMemoryPromptRegistry,
    callbacks: &mut RecordingCallbacks,
    prompt_id: &str,
    environment: &str,
) {
    let prompt = prompt_spec(prompt_id, environment, "dotprompt");
    library
        .initialize_prompt(&prompt, None, callbacks)
        .expect("the fixture prompt registers");
}

#[test]
fn deletes_every_version_of_a_base_id() {
    let first = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&first, &mut callbacks);
    registered(&mut registry, &mut callbacks, "greeting.v2", "development");
    registered(&mut registry, &mut callbacks, "other.v1", "development");
    callbacks.calls.clear();
    let deleted = registry.delete_prompts_by_base_id("greeting", None, &mut callbacks);
    assert_eq!(
        deleted,
        vec![
            "greeting.v1::development".to_owned(),
            "greeting.v2::development".to_owned(),
        ]
    );
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
fn removes_the_callback_of_every_deleted_prompt() {
    let mut first = prompt_spec("greeting.v1", "development", "dotprompt");
    first.litellm_params.prompt_id = Some("first".to_owned());
    let mut second = prompt_spec("greeting.v2", "development", "dotprompt");
    second.litellm_params.prompt_id = Some("second".to_owned());
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&first, &mut callbacks);
    registry
        .initialize_prompt(&second, None, &mut callbacks)
        .expect("the second prompt registers");
    callbacks.calls.clear();
    registry.delete_prompts_by_base_id("greeting", None, &mut callbacks);
    assert_eq!(
        callbacks.calls,
        [
            CallbackCall::Removed(client(Some("first"))),
            CallbackCall::Removed(client(Some("second"))),
        ]
    );
}

#[test]
fn an_environment_filter_limits_the_deletion() {
    let development = prompt_spec("greeting.v1", "development", "dotprompt");
    let production = prompt_spec("greeting.v2", "production", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&development, &mut callbacks);
    registry
        .initialize_prompt(&production, None, &mut callbacks)
        .expect("the production prompt registers");
    callbacks.calls.clear();
    let deleted =
        registry.delete_prompts_by_base_id("greeting", Some("production"), &mut callbacks);
    assert_eq!(deleted, vec!["greeting.v2::production".to_owned()]);
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v1::development")
    );
    assert!(
        registry
            .prompt_id_to_custom_prompt
            .contains_key("greeting.v1::development")
    );
}

#[test]
fn without_an_environment_filter_deletes_across_environments() {
    let development = prompt_spec("greeting.v1", "development", "dotprompt");
    let production = prompt_spec("greeting.v1", "production", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&development, &mut callbacks);
    registry
        .initialize_prompt(&production, None, &mut callbacks)
        .expect("the production prompt registers");
    callbacks.calls.clear();
    let deleted = registry.delete_prompts_by_base_id("greeting", None, &mut callbacks);
    assert_eq!(
        deleted,
        vec![
            "greeting.v1::development".to_owned(),
            "greeting.v1::production".to_owned(),
        ]
    );
    assert!(registry.in_memory_prompts.is_empty());
    assert!(registry.prompt_id_to_custom_prompt.is_empty());
}

#[test]
fn an_unmatched_base_id_deletes_nothing() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let deleted = registry.delete_prompts_by_base_id("missing", None, &mut callbacks);
    assert!(deleted.is_empty());
    assert!(callbacks.calls.is_empty());
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert_eq!(registry.prompt_id_to_custom_prompt.len(), 1);
}

#[test]
fn an_underscore_version_matches_the_same_base() {
    let prompt = prompt_spec("greeting_v2", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let deleted = registry.delete_prompts_by_base_id("greeting", None, &mut callbacks);
    assert_eq!(deleted, vec!["greeting_v2::development".to_owned()]);
    assert!(registry.in_memory_prompts.is_empty());
}

#[test]
fn the_filter_reads_the_spec_environment_not_the_prompt_info() {
    let mut prompt = prompt_spec("greeting.v1", "production", "dotprompt");
    prompt.prompt_info.environment = "development".to_owned();
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let deleted =
        registry.delete_prompts_by_base_id("greeting", Some("production"), &mut callbacks);
    assert_eq!(deleted, vec!["greeting.v1::production".to_owned()]);
    assert!(registry.in_memory_prompts.is_empty());
}

#[test]
fn an_empty_environment_filter_matches_only_an_empty_environment() {
    let empty = prompt_spec("greeting.v1", "", "dotprompt");
    let development = prompt_spec("greeting.v2", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&empty, &mut callbacks);
    registry
        .initialize_prompt(&development, None, &mut callbacks)
        .expect("the development prompt registers");
    callbacks.calls.clear();
    let deleted = registry.delete_prompts_by_base_id("greeting", Some(""), &mut callbacks);
    assert_eq!(deleted, vec!["greeting.v1::".to_owned()]);
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v2::development")
    );
}

#[test]
fn a_key_without_a_callback_is_still_deleted() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .in_memory_prompts
        .insert(registry_key.clone(), prompt);
    let deleted = registry.delete_prompts_by_base_id("greeting", None, &mut callbacks);
    assert_eq!(deleted, vec![registry_key]);
    assert!(registry.in_memory_prompts.is_empty());
    assert!(callbacks.calls.is_empty());
}
