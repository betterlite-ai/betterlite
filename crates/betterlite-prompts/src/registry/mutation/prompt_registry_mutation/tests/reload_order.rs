#![cfg(test)]

use betterlite_models::prompt_spec::PromptSpec;

use super::support::{CallbackCall, RecordingCallbacks, client, prompt_spec, registry_with};
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;

fn bitbucket_spec() -> PromptSpec {
    let mut prompt = prompt_spec("greeting.v1", "development", "bitbucket");
    prompt.litellm_params.prompt_id = Some("old-repo".to_owned());
    prompt
}

fn generic_spec() -> PromptSpec {
    let mut prompt = prompt_spec("greeting.v1", "development", "generic_prompt_management");
    prompt.litellm_params.prompt_id = Some("new-repo".to_owned());
    prompt
}

#[test]
fn a_reload_removes_the_stale_callback_then_adds_the_new_one() {
    let prompt = bitbucket_spec();
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    registry
        .reload_prompt(&generic_spec(), &mut callbacks)
        .expect("the replacement builds");
    assert_eq!(
        callbacks.calls,
        [
            CallbackCall::Removed(client(Some("old-repo"))),
            CallbackCall::Added(client(Some("new-repo"))),
        ]
    );
}

#[test]
fn a_reload_updates_both_maps_under_the_same_key() {
    let prompt = bitbucket_spec();
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    registry
        .reload_prompt(&generic_spec(), &mut callbacks)
        .expect("the replacement builds");
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert_eq!(registry.prompt_id_to_custom_prompt.len(), 1);
    assert_eq!(
        registry
            .in_memory_prompts
            .get(&registry_key)
            .map(|stored| stored.litellm_params.prompt_integration.clone()),
        Some("generic_prompt_management".to_owned())
    );
    assert_eq!(
        registry.prompt_id_to_custom_prompt.get(&registry_key),
        Some(&client(Some("new-repo")))
    );
}

#[test]
fn a_reload_returns_the_parsed_prompt() {
    let prompt = bitbucket_spec();
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let parsed = registry
        .reload_prompt(&generic_spec(), &mut callbacks)
        .expect("the replacement builds");
    assert_eq!(parsed.prompt_id, "greeting.v1");
    assert_eq!(
        parsed.litellm_params.prompt_integration,
        "generic_prompt_management".to_owned()
    );
}

#[test]
fn a_reload_replaces_a_stored_prompt_even_when_the_values_match() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut changed = prompt.clone();
    changed.version = Some(2);
    let parsed = registry
        .reload_prompt(&changed, &mut callbacks)
        .expect("the replacement builds");
    assert_eq!(parsed.version, Some(2));
    assert_eq!(
        registry
            .in_memory_prompts
            .get(&registry_key_for_prompt(&prompt))
            .map(|stored| stored.version),
        Some(Some(2))
    );
}

#[test]
fn a_reload_of_an_absent_key_adds_without_removing() {
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .reload_prompt(&generic_spec(), &mut callbacks)
        .expect("an absent key registers");
    assert_eq!(
        callbacks.calls,
        [CallbackCall::Added(client(Some("new-repo")))]
    );
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert_eq!(registry.prompt_id_to_custom_prompt.len(), 1);
}

#[test]
fn a_reload_moves_the_key_to_the_end_of_both_maps() {
    let greeting = prompt_spec("greeting.v1", "development", "dotprompt");
    let other = prompt_spec("other.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&greeting, &mut callbacks);
    registry
        .initialize_prompt(&other, None, &mut callbacks)
        .expect("the second prompt registers");
    registry
        .reload_prompt(&greeting, &mut callbacks)
        .expect("the replacement builds");
    assert_eq!(
        registry
            .in_memory_prompts
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["other.v1::development", "greeting.v1::development"]
    );
    assert_eq!(
        registry
            .prompt_id_to_custom_prompt
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["other.v1::development", "greeting.v1::development"]
    );
}
