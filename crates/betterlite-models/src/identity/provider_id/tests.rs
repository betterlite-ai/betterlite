#![cfg(test)]

use super::ProviderId;
use rstest::rstest;

#[test]
fn every_variant_round_trips_through_its_wire_slug() {
    assert!(
        ProviderId::ALL
            .iter()
            .all(|provider| ProviderId::parse(provider.as_str()) == Some(*provider))
    );
}

#[test]
fn has_the_full_llm_providers_count() {
    assert_eq!(ProviderId::ALL.len(), 165);
}

#[rstest]
#[case::rejects_an_unknown_slug("not-a-real-provider", None)]
#[case::parse_is_case_sensitive("OpenAI", None)]
#[case::exact_slug_resolves("openai", Some(ProviderId::Openai))]
#[case::a_slug_that_does_not_mirror_its_variant_name_still_resolves(
    "meta_llama",
    Some(ProviderId::Llama)
)]
#[case::a_hyphenated_slug_resolves(
    "text-completion-openai",
    Some(ProviderId::TextCompletionOpenai)
)]
#[case::another_hyphenated_slug_resolves("nano-gpt", Some(ProviderId::Nanogpt))]
#[case::a_slug_collapsed_from_a_longer_variant_name_resolves(
    "sap",
    Some(ProviderId::SapGenerativeAiHub)
)]
fn provider_identity_behavior_tests_behavior(
    #[case] slug: &str,
    #[case] expected: Option<ProviderId>,
) {
    assert_eq!(ProviderId::parse(slug), expected);
}

#[test]
fn displays_as_its_wire_slug() {
    assert_eq!(ProviderId::Cohere.to_string(), "cohere");
}
