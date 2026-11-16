#![cfg(test)]

use std::path::Path;

use betterlite_models::prompt_spec::PromptInfo;

use super::support::{CallbackCall, RecordingCallbacks, client, prompt_spec};
use crate::registry::mutation::prompt_registry_mutation::error::PromptRegistryError;
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;

fn timestamp(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text)
        .expect("the fixture timestamp is valid")
        .with_timezone(&chrono::Utc)
}

#[test]
fn stores_the_parsed_prompt_and_the_callback_under_the_registry_key() {
    let prompt = prompt_spec("greeting.v1", "production", "dotprompt");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    let parsed = registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the prompt registers");
    assert_eq!(parsed.prompt_id, "greeting.v1");
    assert_eq!(
        registry
            .in_memory_prompts
            .get(&registry_key)
            .map(|stored| stored.prompt_id.clone()),
        Some("greeting.v1".to_owned())
    );
    assert_eq!(
        registry.prompt_id_to_custom_prompt.get(&registry_key),
        Some(&client(Some("greeting")))
    );
}

#[test]
fn registers_the_callback_with_the_logging_manager() {
    let prompt = prompt_spec("greeting.v1", "production", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the prompt registers");
    assert_eq!(
        callbacks.calls,
        [CallbackCall::Added(client(Some("greeting")))]
    );
}

#[test]
fn returns_a_parsed_prompt_that_copies_every_field_of_the_input() {
    let mut prompt = prompt_spec("greeting.v1", "staging", "gitlab");
    prompt.created_at = Some(timestamp("2024-01-02T03:04:05Z"));
    prompt.updated_at = Some(timestamp("2024-02-03T04:05:06Z"));
    prompt.version = Some(7);
    prompt.created_by = Some("someone".to_owned());
    prompt.prompt_info = PromptInfo {
        prompt_type: "db".to_owned(),
        environment: "production".to_owned(),
        extra: indexmap::IndexMap::new(),
    };
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    let parsed = registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the prompt registers");
    assert_eq!(parsed.prompt_id, prompt.prompt_id);
    assert_eq!(parsed.created_at, prompt.created_at);
    assert_eq!(parsed.updated_at, prompt.updated_at);
    assert_eq!(parsed.version, Some(7));
    assert_eq!(parsed.created_by, Some("someone".to_owned()));
    assert_eq!(parsed.prompt_info.prompt_type, "db");
    assert_eq!(parsed.prompt_info.environment, "production");
    assert_eq!(parsed.litellm_params.prompt_integration, "gitlab");
}

#[test]
fn keys_by_both_the_prompt_id_and_the_environment() {
    let development = prompt_spec("greeting.v1", "development", "dotprompt");
    let production = prompt_spec("greeting.v1", "production", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .initialize_prompt(&development, None, &mut callbacks)
        .expect("the development prompt registers");
    registry
        .initialize_prompt(&production, None, &mut callbacks)
        .expect("the production prompt registers");
    assert_eq!(registry.in_memory_prompts.len(), 2);
    assert_eq!(registry.prompt_id_to_custom_prompt.len(), 2);
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v1::development")
    );
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v1::production")
    );
}

#[test]
fn a_config_file_path_is_accepted_and_ignored() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let path = Path::new("/does/not/exist.prompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    let parsed = registry
        .initialize_prompt(&prompt, Some(path), &mut callbacks)
        .expect("the path is not read");
    assert_eq!(parsed.prompt_id, "greeting.v1");
}

#[test]
fn two_registries_hold_separate_prompts() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut first = InMemoryPromptRegistry::default();
    let second = InMemoryPromptRegistry::default();
    first
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect("the prompt registers");
    assert_eq!(first.in_memory_prompts.len(), 1);
    assert!(second.in_memory_prompts.is_empty());
    assert!(second.prompt_id_to_custom_prompt.is_empty());
}

#[test]
fn an_unsupported_integration_registers_nothing() {
    let prompt = prompt_spec("greeting.v1", "production", "langfuse");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = InMemoryPromptRegistry::default();
    let error = registry
        .initialize_prompt(&prompt, None, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert!(matches!(
        error,
        PromptRegistryError::UnsupportedIntegration { integration } if integration == "langfuse"
    ));
    assert!(registry.in_memory_prompts.is_empty());
    assert!(registry.prompt_id_to_custom_prompt.is_empty());
    assert!(callbacks.calls.is_empty());
}
