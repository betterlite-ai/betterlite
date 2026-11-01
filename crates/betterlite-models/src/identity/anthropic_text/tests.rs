#![cfg(test)]

use super::anthropic_text_provider;
use rstest::rstest;

#[rstest]
#[case::declared_anthropic_provider_with_claude_2(
    "claude-2",
    Some("anthropic"),
    "claude-2",
    Some("anthropic_text")
)]
#[case::declared_anthropic_provider_with_claude_instant_1(
    "claude-instant-1",
    Some("anthropic"),
    "claude-instant-1",
    Some("anthropic_text")
)]
#[case::declared_anthropic_provider_with_a_non_text_model(
    "claude-3-opus",
    Some("anthropic"),
    "claude-3-opus",
    Some("anthropic")
)]
#[case::prefixed_model_with_no_declared_provider(
    "anthropic/claude-2",
    None,
    "claude-2",
    Some("anthropic_text")
)]
#[case::prefixed_model_with_a_mismatched_declared_provider(
    "anthropic/claude-2",
    Some("azure"),
    "claude-2",
    Some("anthropic_text")
)]
#[case::prefixed_but_not_a_text_model_passes_through(
    "anthropic/claude-3-opus",
    None,
    "anthropic/claude-3-opus",
    None
)]
#[case::unrelated_model_passes_through_untouched("gpt-4", Some("openai"), "gpt-4", Some("openai"))]
fn provider_identity_behavior_tests_behavior(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected_model: &str,
    #[case] expected_provider: Option<&str>,
) {
    let (actual_model, actual_provider) = anthropic_text_provider(model, custom_llm_provider);
    assert_eq!(actual_model, expected_model);
    assert_eq!(actual_provider.as_deref(), expected_provider);
}
