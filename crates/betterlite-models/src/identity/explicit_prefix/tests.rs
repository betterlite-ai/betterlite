#![cfg(test)]

use super::{PrefixClassification, PrefixClassificationInput, classify_prefix};
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::identity::provider_id::ProviderId;
use rstest::rstest;
use std::collections::BTreeSet;

fn synthetic_catalog() -> ModelCatalog {
    ModelCatalog::parse(
        br#"{
            "command-r": {"litellm_provider": "cohere_chat"},
            "claude-2": {"litellm_provider": "anthropic"},
            "gpt-4": {"litellm_provider": "openai"}
        }"#,
        UnknownFields::Reject,
    )
    .unwrap()
}

fn classify(
    model: &str,
    custom_llm_provider: Option<&str>,
    configured_model_names: &BTreeSet<String>,
) -> PrefixClassification {
    let catalog = synthetic_catalog();
    classify_prefix(&PrefixClassificationInput {
        model,
        custom_llm_provider,
        configured_model_names,
        catalog: &catalog,
    })
}

#[rstest]
#[case::azure_proxies_a_cohere_chat_model(
    "azure/command-r",
    None,
    PrefixClassification::NonOpenaiAzure { model: "azure/command-r".to_owned() }
)]
#[case::explicit_provider_prefix_needs_defaults(
    "mistral/mistral-small",
    None,
    PrefixClassification::ExplicitProvider {
        provider: ProviderId::Mistral,
        model: "mistral-small".to_owned(),
        needs_provider_defaults: true,
    }
)]
#[case::openrouter_native_id_stays_intact(
    "openrouter/auto",
    Some("openrouter"),
    PrefixClassification::OpenRouterNative { model: "openrouter/auto".to_owned() }
)]
#[case::openrouter_nested_id_is_normalized(
    "openrouter/anthropic/claude-3.5-sonnet",
    Some("openrouter"),
    PrefixClassification::OpenRouterNested { model: "anthropic/claude-3.5-sonnet".to_owned() }
)]
#[case::cohere_chat_normalization_runs_before_the_explicit_check(
    "cohere/command-r",
    None,
    PrefixClassification::ExplicitProvider {
        provider: ProviderId::CohereChat,
        model: "command-r".to_owned(),
        needs_provider_defaults: true,
    }
)]
#[case::anthropic_text_normalization_runs_before_the_explicit_check(
    "anthropic/claude-2",
    None,
    PrefixClassification::ExplicitProvider {
        provider: ProviderId::AnthropicText,
        model: "claude-2".to_owned(),
        needs_provider_defaults: true,
    }
)]
#[case::declared_provider_that_mismatches_the_model_prefix_is_reinserted(
    "gpt-4",
    Some("openai"),
    PrefixClassification::ExplicitProvider {
        provider: ProviderId::Openai,
        model: "gpt-4".to_owned(),
        needs_provider_defaults: true,
    }
)]
#[case::a_bare_model_name_with_no_provider_prefix_is_unresolved(
    "gpt-4",
    None,
    PrefixClassification::Unresolved { model: "gpt-4".to_owned(), custom_llm_provider: None }
)]
#[case::an_unknown_prefix_is_unresolved(
    "my-custom-provider/some-model",
    None,
    PrefixClassification::Unresolved {
        model: "my-custom-provider/some-model".to_owned(),
        custom_llm_provider: None,
    }
)]
fn provider_identity_behavior_tests_behavior(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected: PrefixClassification,
) {
    let configured = BTreeSet::new();
    assert_eq!(classify(model, custom_llm_provider, &configured), expected);
}

#[test]
fn a_provider_prefix_that_is_also_a_configured_model_name_does_not_need_defaults() {
    let mut configured = BTreeSet::new();
    configured.insert("mistral".to_owned());
    assert_eq!(
        classify("mistral/mistral-small", None, &configured),
        PrefixClassification::ExplicitProvider {
            provider: ProviderId::Mistral,
            model: "mistral-small".to_owned(),
            needs_provider_defaults: false,
        }
    );
}
