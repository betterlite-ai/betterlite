#![cfg(test)]

use super::cohere_chat_provider;
use crate::catalog::{ModelCatalog, UnknownFields};
use rstest::rstest;

fn synthetic_catalog() -> ModelCatalog {
    ModelCatalog::parse(
        br#"{
            "command-r": {"litellm_provider": "cohere_chat"},
            "embed-english-v3.0": {"litellm_provider": "cohere"}
        }"#,
        UnknownFields::Reject,
    )
    .unwrap()
}

#[rstest]
#[case::declared_cohere_provider_with_a_known_chat_model(
    "command-r",
    Some("cohere"),
    "command-r",
    Some("cohere_chat")
)]
#[case::declared_cohere_provider_with_a_non_chat_model(
    "embed-english-v3.0",
    Some("cohere"),
    "embed-english-v3.0",
    Some("cohere")
)]
#[case::prefixed_model_with_no_declared_provider(
    "cohere/command-r",
    None,
    "command-r",
    Some("cohere_chat")
)]
#[case::prefixed_model_with_a_mismatched_declared_provider(
    "cohere/command-r",
    Some("azure"),
    "command-r",
    Some("cohere_chat")
)]
#[case::prefixed_but_unknown_chat_model_passes_through(
    "cohere/embed-english-v3.0",
    None,
    "cohere/embed-english-v3.0",
    None
)]
#[case::unrelated_model_passes_through_untouched("gpt-4", Some("openai"), "gpt-4", Some("openai"))]
fn provider_identity_behavior_tests_behavior(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected_model: &str,
    #[case] expected_provider: Option<&str>,
) {
    let catalog = synthetic_catalog();
    let (actual_model, actual_provider) =
        cohere_chat_provider(model, custom_llm_provider, &catalog);
    assert_eq!(actual_model, expected_model);
    assert_eq!(actual_provider.as_deref(), expected_provider);
}
