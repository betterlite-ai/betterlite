#![cfg(test)]

use super::authenticating_provider;
use crate::identity::provider_id::ProviderId;
use rstest::rstest;

#[rstest]
#[case::declared_provider_is_github_copilot(
    Some("gpt-4"),
    Some("github_copilot"),
    Some(ProviderId::GithubCopilot)
)]
#[case::declared_provider_is_chatgpt(Some("gpt-4"), Some("chatgpt"), Some(ProviderId::Chatgpt))]
#[case::declared_provider_is_not_an_authenticating_one(Some("gpt-4"), Some("openai"), None)]
#[case::prefix_names_an_authenticating_provider(
    Some("github_copilot/gpt-4"),
    None,
    Some(ProviderId::GithubCopilot)
)]
#[case::prefix_names_a_non_authenticating_provider(Some("openai/gpt-4"), None, None)]
#[case::no_model_and_no_custom_provider(None, None, None)]
#[case::model_with_no_slash_and_no_custom_provider(Some("gpt-4"), None, None)]
#[case::unknown_custom_provider_slug(Some("gpt-4"), Some("not-a-provider"), None)]
#[case::inferred_shortcut_matches_a_declared_prefix(
    Some("chatgpt/gpt-4"),
    None,
    Some(ProviderId::Chatgpt)
)]
#[case::inferred_shortcut_is_none_for_a_non_authenticating_prefix(Some("openai/gpt-4"), None, None)]
fn provider_identity_behavior_tests_behavior(
    #[case] model: Option<&str>,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected: Option<ProviderId>,
) {
    assert_eq!(
        authenticating_provider(model, custom_llm_provider),
        expected
    );
}
