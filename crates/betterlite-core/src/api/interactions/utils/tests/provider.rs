use rstest::rstest;

use crate::api::interactions::utils::selection::{
    InteractionsProvider, select_interactions_provider,
};

#[rstest]
#[case::gemini(Some("gemini"), Some(InteractionsProvider::GoogleAiStudio))]
#[case::vertex(Some("vertex_ai"), Some(InteractionsProvider::VertexAi))]
#[case::vertex_beta(Some("vertex_ai_beta"), Some(InteractionsProvider::VertexAi))]
#[case::empty(Some(""), None)]
#[case::absent(None, None)]
#[case::unsupported(Some("openai"), None)]
#[case::unrelated_prefix(Some("gemini-pro"), None)]
#[case::wrong_case(Some("Gemini"), None)]
#[case::litellm_responses(Some("litellm_responses"), None)]
fn selects_the_provider_config(
    #[case] provider: Option<&str>,
    #[case] expected: Option<InteractionsProvider>,
) {
    assert_eq!(select_interactions_provider(provider), expected);
}

#[test]
fn gemini_and_vertex_select_different_providers() {
    assert_ne!(
        select_interactions_provider(Some("gemini")),
        select_interactions_provider(Some("vertex_ai"))
    );
}
