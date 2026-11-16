#![cfg(test)]

use betterlite_models::prompt_spec::SupportedPromptIntegrations;
use rstest::rstest;

use super::initializers::{PROMPT_INITIALIZER_REGISTRY, prompt_initializer};

// Reading the length in a const proves the registry is built at compile time, not filled by a call
const POPULATED_AT_COMPILE_TIME: usize = PROMPT_INITIALIZER_REGISTRY.len();

#[test]
fn the_registry_holds_one_initializer_per_supported_integration() {
    assert_eq!(POPULATED_AT_COMPILE_TIME, 4);
    assert_eq!(PROMPT_INITIALIZER_REGISTRY.len(), 4);
}

#[rstest]
#[case::dotprompt("dotprompt", SupportedPromptIntegrations::DotPrompt)]
#[case::bitbucket("bitbucket", SupportedPromptIntegrations::Bitbucket)]
#[case::gitlab("gitlab", SupportedPromptIntegrations::Gitlab)]
#[case::generic(
    "generic_prompt_management",
    SupportedPromptIntegrations::GenericPromptManagement
)]
fn the_registry_is_keyed_by_the_litellm_integration_name(
    #[case] name: &str,
    #[case] integration: SupportedPromptIntegrations,
) {
    assert!(
        PROMPT_INITIALIZER_REGISTRY
            .iter()
            .any(|(registered, _)| *registered == name)
    );
    assert!(prompt_initializer(name).is_some());
    let decoded: SupportedPromptIntegrations =
        serde_json::from_str(&format!("\"{name}\"")).expect("the name is a LiteLLM integration");
    assert_eq!(decoded, integration);
}

#[test]
fn the_registry_keeps_one_entry_per_name() {
    let mut names: Vec<&str> = PROMPT_INITIALIZER_REGISTRY
        .iter()
        .map(|(name, _)| *name)
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), PROMPT_INITIALIZER_REGISTRY.len());
}

#[test]
fn an_empty_name_finds_no_initializer() {
    assert!(prompt_initializer("").is_none());
}
