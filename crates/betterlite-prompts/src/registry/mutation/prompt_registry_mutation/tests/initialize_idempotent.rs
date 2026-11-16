#![cfg(test)]

use super::support::{CallbackCall, RecordingCallbacks, client, prompt_spec};
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;

#[test]
fn a_repeat_call_returns_the_stored_spec() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    let first = registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the first call registers");
    let second = registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the second call returns the stored prompt");
    assert_eq!(first.prompt_id, second.prompt_id);
    assert_eq!(second.created_by, first.created_by);
}

#[test]
fn a_repeat_call_adds_no_second_callback() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the first call registers");
    registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the second call returns the stored prompt");
    assert_eq!(
        callbacks.calls,
        [CallbackCall::Added(client(Some("greeting")))]
    );
}

#[test]
fn the_stored_spec_wins_over_the_new_input() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut changed = prompt_spec("greeting.v1", "development", "dotprompt");
    changed.created_by = Some("someone-else".to_owned());
    changed.version = Some(9);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the first call registers");
    let returned = registry
        .initialize_prompt(&changed, None, &mut callbacks)
        .expect("the second call returns the stored prompt");
    assert_eq!(returned.created_by, None);
    assert_eq!(returned.version, None);
    assert_eq!(
        registry
            .in_memory_prompts
            .get(&registry_key_for_prompt(&prompt))
            .map(|stored| stored.version),
        Some(None)
    );
}

#[test]
fn a_repeat_call_keeps_the_first_client() {
    let mut prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    prompt.litellm_params.prompt_id = Some("first".to_owned());
    let mut changed = prompt.clone();
    changed.litellm_params.prompt_id = Some("second".to_owned());
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the first call registers");
    registry
        .initialize_prompt(&changed, None, &mut callbacks)
        .expect("the second call returns the stored prompt");
    assert_eq!(
        registry
            .prompt_id_to_custom_prompt
            .get(&registry_key_for_prompt(&prompt)),
        Some(&client(Some("first")))
    );
}
