#![cfg(test)]

use super::ProviderId;
use rstest::rstest;

#[test]
fn every_variant_round_trips_through_its_wire_slug() {
    assert!(ProviderId::ALL.iter().all(|provider| {
        let slug = <&str>::from(*provider);
        slug.parse::<ProviderId>() == Ok(*provider)
    }));
}

#[test]
fn has_the_full_llm_providers_count_plus_the_json_registry_only_slugs() {
    assert_eq!(ProviderId::ALL.len(), 165 + 8);
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
    assert_eq!(slug.parse::<ProviderId>().ok(), expected);
}

#[test]
fn displays_as_its_wire_slug() {
    assert_eq!(ProviderId::Cohere.to_string(), "cohere");
}
