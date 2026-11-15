#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::PromptSpec;

use super::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::registry_key_for_prompt;
use crate::tests::prompt_spec;

fn callback(prompt_id: &str) -> PromptManagementClient {
    PromptManagementClient {
        prompt_id: Some(prompt_id.to_owned()),
        prompt_template: vec![Json::Text("hello".to_owned())],
        prompt_template_model: None,
        prompt_optional_params: None,
        completed_messages: None,
    }
}

fn registry_with(prompts: Vec<PromptSpec>) -> InMemoryPromptRegistry {
    let mut registry = InMemoryPromptRegistry::default();
    for prompt in prompts {
        let registry_key = registry_key_for_prompt(&prompt);
        registry.in_memory_prompts.insert(registry_key, prompt);
    }
    registry
}

#[test]
fn the_registry_keys_both_maps_by_prompt_id_and_environment() {
    let prompt = prompt_spec("greeting.v1", Some(1), "production");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .in_memory_prompts
        .insert(registry_key.clone(), prompt);
    registry
        .prompt_id_to_custom_prompt
        .insert(registry_key.clone(), callback("greeting.v1"));
    assert_eq!(registry_key, "greeting.v1::production");
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert_eq!(registry.prompt_id_to_custom_prompt.len(), 1);
    assert!(registry.in_memory_prompts.contains_key(&registry_key));
    assert!(
        registry
            .prompt_id_to_custom_prompt
            .contains_key(&registry_key)
    );
}

#[test]
fn resolve_prompt_spec_returns_the_highest_version_in_the_environment() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v3", Some(3), "development"),
        prompt_spec("greeting.v2", Some(2), "development"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.prompt_id, "greeting.v3");
}

#[test]
fn resolve_prompt_spec_returns_the_requested_version() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v2", Some(2), "development"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", Some(1), None)
        .expect("version 1 resolves");
    assert_eq!(resolved.prompt_id, "greeting.v1");
}

#[test]
fn resolve_prompt_spec_returns_none_for_a_missing_version() {
    let registry = registry_with(vec![prompt_spec("greeting.v1", Some(1), "development")]);
    assert!(
        registry
            .resolve_prompt_spec("greeting", Some(3), None)
            .is_none()
    );
}

#[test]
fn resolve_prompt_spec_prefers_production_over_staging_and_development() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v1", Some(1), "staging"),
        prompt_spec("greeting.v1", Some(1), "production"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.environment, "production");
}

#[test]
fn resolve_prompt_spec_prefers_staging_over_development() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v1", Some(1), "staging"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.environment, "staging");
}

#[test]
fn resolve_prompt_spec_falls_back_to_the_alphabetically_first_environment() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "qa"),
        prompt_spec("greeting.v1", Some(1), "canary"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.environment, "canary");
}

#[test]
fn resolve_prompt_spec_uses_the_explicit_environment() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v2", Some(2), "production"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, Some("development"))
        .expect("development resolves");
    assert_eq!(resolved.prompt_id, "greeting.v1");
}

#[test]
fn resolve_prompt_spec_returns_none_for_a_missing_environment() {
    let registry = registry_with(vec![prompt_spec("greeting.v1", Some(1), "development")]);
    assert!(
        registry
            .resolve_prompt_spec("greeting", None, Some("production"))
            .is_none()
    );
}

#[test]
fn resolve_prompt_spec_returns_none_for_an_unknown_base_id() {
    let registry = registry_with(vec![prompt_spec("greeting.v1", Some(1), "development")]);
    assert!(registry.resolve_prompt_spec("other", None, None).is_none());
}

#[test]
fn resolve_prompt_spec_strips_the_version_from_the_requested_id() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v3", Some(3), "development"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting.v2", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.prompt_id, "greeting.v3");
}

#[test]
fn resolve_prompt_spec_prefers_the_explicit_version_field() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(7), "development"),
        prompt_spec("greeting.v2", None, "development"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.prompt_id, "greeting.v1");
}

#[test]
fn resolve_prompt_spec_keeps_the_first_of_two_equal_versions() {
    let registry = registry_with(vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v2", Some(1), "development"),
    ]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.prompt_id, "greeting.v1");
}

#[test]
fn resolve_prompt_spec_matches_an_underscore_suffixed_prompt_id() {
    let registry = registry_with(vec![prompt_spec("greeting_v2", Some(2), "development")]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.prompt_id, "greeting_v2");
}

#[test]
fn resolve_prompt_spec_matches_a_dot_suffixed_prompt_id() {
    let registry = registry_with(vec![prompt_spec("greeting.v2", Some(2), "development")]);
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(resolved.prompt_id, "greeting.v2");
}

#[test]
fn has_config_prompt_matches_a_versioned_config_prompt() {
    let mut config_prompt = prompt_spec("greeting.v1", Some(1), "development");
    config_prompt.prompt_info.prompt_type = "config".to_owned();
    let registry = registry_with(vec![
        config_prompt,
        prompt_spec("greeting.v1", Some(1), "production"),
    ]);
    assert!(registry.has_config_prompt("greeting"));
    assert!(!registry.has_config_prompt("other"));
}

#[test]
fn has_config_prompt_is_false_for_database_prompts() {
    let mut database_prompt = prompt_spec("greeting.v1", Some(1), "development");
    database_prompt.prompt_info.prompt_type = "db".to_owned();
    let registry = registry_with(vec![database_prompt]);
    assert!(!registry.has_config_prompt("greeting"));
}

#[test]
fn get_prompt_callback_for_prompt_finds_the_callback_by_registry_key() {
    let prompt = prompt_spec("greeting.v1", Some(1), "production");
    let registry_key = registry_key_for_prompt(&prompt);
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .in_memory_prompts
        .insert(registry_key.clone(), prompt);
    registry
        .prompt_id_to_custom_prompt
        .insert(registry_key, callback("greeting.v1"));
    let resolved = registry
        .resolve_prompt_spec("greeting", None, None)
        .expect("greeting resolves");
    assert_eq!(
        registry.get_prompt_callback_for_prompt(&resolved),
        Some(callback("greeting.v1"))
    );
}

#[test]
fn get_prompt_callback_for_prompt_returns_none_for_an_unknown_key() {
    let registry = InMemoryPromptRegistry::default();
    let prompt = prompt_spec("greeting.v1", Some(1), "production");
    assert!(registry.get_prompt_callback_for_prompt(&prompt).is_none());
}
