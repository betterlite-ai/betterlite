#![cfg(test)]

use super::{is_azure_claude_model, is_non_openai_azure_model};
use crate::catalog::{ModelCatalog, UnknownFields};
use rstest::rstest;

fn synthetic_catalog() -> ModelCatalog {
    ModelCatalog::parse(
        br#"{
            "command-a-03-2025": {"litellm_provider": "cohere_chat"},
            "mistral/codestral-2508": {"litellm_provider": "mistral"},
            "gpt-4": {"litellm_provider": "openai"}
        }"#,
        UnknownFields::Reject,
    )
    .unwrap()
}

enum Check {
    NonOpenaiAzure(&'static str),
    AzureClaude(&'static str),
}

#[rstest]
#[case::cohere_chat_model_behind_azure(Check::NonOpenaiAzure("azure/command-a-03-2025"), true)]
#[case::mistral_model_behind_azure(Check::NonOpenaiAzure("azure/codestral-2508"), true)]
#[case::true_openai_model_behind_azure(Check::NonOpenaiAzure("azure/gpt-4"), false)]
#[case::model_not_in_the_catalog_at_all(Check::NonOpenaiAzure("azure/unknown-model"), false)]
#[case::no_slash_at_all(Check::NonOpenaiAzure("command-a-03-2025"), false)]
#[case::exact_claude_name(Check::AzureClaude("claude-3-opus"), true)]
#[case::claude_as_a_substring(Check::AzureClaude("my-claude-model"), true)]
#[case::claude_check_is_case_insensitive(Check::AzureClaude("CLAUDE-3"), true)]
#[case::claude_check_rejects_an_unrelated_model(Check::AzureClaude("gpt-4"), false)]
fn provider_identity_behavior_tests_behavior(#[case] check: Check, #[case] expected: bool) {
    let catalog = synthetic_catalog();
    let actual = match check {
        Check::NonOpenaiAzure(model) => is_non_openai_azure_model(model, &catalog),
        Check::AzureClaude(model) => is_azure_claude_model(model),
    };
    assert_eq!(actual, expected);
}
