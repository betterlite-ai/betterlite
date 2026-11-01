#![cfg(test)]

use super::{OpenRouterPrefix, normalize_openrouter_prefix};
use rstest::rstest;

#[rstest]
#[case::native_id_stays_intact(
    "openrouter/auto",
    Some("openrouter"),
    OpenRouterPrefix::Native { model: "openrouter/auto".to_owned() }
)]
#[case::nested_id_is_stripped(
    "openrouter/anthropic/claude-3.5-sonnet",
    Some("openrouter"),
    OpenRouterPrefix::Nested { model: "anthropic/claude-3.5-sonnet".to_owned() }
)]
#[case::wrong_declared_provider_is_not_applicable(
    "openrouter/auto",
    Some("azure"),
    OpenRouterPrefix::NotApplicable
)]
#[case::no_declared_provider_is_not_applicable(
    "openrouter/auto",
    None,
    OpenRouterPrefix::NotApplicable
)]
#[case::model_without_the_openrouter_prefix_is_not_applicable(
    "anthropic/claude-3.5-sonnet",
    Some("openrouter"),
    OpenRouterPrefix::NotApplicable
)]
fn provider_identity_behavior_tests_behavior(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected: OpenRouterPrefix,
) {
    assert_eq!(
        normalize_openrouter_prefix(model, custom_llm_provider),
        expected
    );
}
