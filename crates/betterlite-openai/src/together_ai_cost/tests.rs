#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::{
    CostMap, PricingCallType, TogetherPricingTiers, model_has_registry_pricing, pricing_category,
};

struct Priced(BTreeSet<String>);

impl CostMap for Priced {
    fn has_price_entry(&self, key: &str) -> bool {
        self.0.contains(key)
    }
}

fn priced(keys: &[&str]) -> Priced {
    Priced(keys.iter().map(|key| String::from(*key)).collect())
}

#[test]
fn a_priced_entry_is_found_through_the_provider_prefix() {
    let map = priced(&["together_ai/llama-3-70b", "openai/gpt-4o"]);
    assert!(model_has_registry_pricing("llama-3-70b", &map));
    assert!(model_has_registry_pricing("together_ai/llama-3-70b", &map));
    assert!(!model_has_registry_pricing("openai/gpt-4o", &map));
    assert!(!model_has_registry_pricing("llama-3-8b", &map));
}

#[test]
fn only_one_provider_prefix_is_stripped() {
    let map = priced(&["together_ai/together_ai/llama"]);
    assert!(model_has_registry_pricing(
        "together_ai/together_ai/llama",
        &map
    ));
}

#[rstest]
#[case::below_4(PricingCallType::Other, "llama-3b", "together-ai-up-to-4b")]
#[case::at_4(PricingCallType::Other, "llama-4b", "together-ai-up-to-4b")]
#[case::above_4(PricingCallType::Other, "llama-5b", "together-ai-4.1b-8b")]
#[case::at_8(PricingCallType::Other, "llama-8b", "together-ai-4.1b-8b")]
#[case::above_8(PricingCallType::Other, "llama-9b", "together-ai-8.1b-21b")]
#[case::at_21(PricingCallType::Other, "llama-21b", "together-ai-8.1b-21b")]
#[case::above_21(PricingCallType::Other, "llama-22b", "together-ai-21.1b-41b")]
#[case::at_41(PricingCallType::Other, "llama-41b", "together-ai-21.1b-41b")]
#[case::above_41(PricingCallType::Other, "llama-42b", "together-ai-41.1b-80b")]
#[case::at_80(PricingCallType::Other, "llama-80b", "together-ai-41.1b-80b")]
#[case::above_80(PricingCallType::Other, "llama-81b", "together-ai-81.1b-110b")]
#[case::at_110(PricingCallType::Other, "llama-110b", "together-ai-81.1b-110b")]
#[case::above_110(PricingCallType::Other, "llama-111b", "llama-111b")]
#[case::huge(
    PricingCallType::Other,
    "llama-999999999999999999999b",
    "llama-999999999999999999999b"
)]
#[case::no_match(PricingCallType::Other, "llama-chat", "llama-chat")]
#[case::detached_suffix(PricingCallType::Other, "llama-70-b", "llama-70-b")]
#[case::first_digits(PricingCallType::Other, "mixtral-8x7b", "together-ai-4.1b-8b")]
#[case::llama_chat(PricingCallType::Other, "llama-3-70b-chat", "together-ai-41.1b-80b")]
#[case::uppercase(PricingCallType::Other, "Llama-3-70B", "together-ai-41.1b-80b")]
#[case::below_150(
    PricingCallType::Embedding,
    "embed-100m",
    "together-ai-embedding-up-to-150m"
)]
#[case::at_150(
    PricingCallType::Embedding,
    "embed-150m",
    "together-ai-embedding-up-to-150m"
)]
#[case::above_150(
    PricingCallType::Embedding,
    "embed-151m",
    "together-ai-embedding-151m-to-350m"
)]
#[case::at_350(
    PricingCallType::Embedding,
    "embed-350m",
    "together-ai-embedding-151m-to-350m"
)]
#[case::above_350(PricingCallType::Embedding, "embed-351m", "embed-351m")]
#[case::embedding_no_match(PricingCallType::Embedding, "embed-model", "embed-model")]
#[case::embedding_uppercase(
    PricingCallType::Embedding,
    "Embed-200M",
    "together-ai-embedding-151m-to-350m"
)]
fn names_map_to_their_price_category(
    #[case] call_type: PricingCallType,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        pricing_category(model, call_type, &TogetherPricingTiers::default()),
        expected
    );
}

#[test]
fn an_aembedding_call_uses_the_embedding_table() {
    assert_eq!(
        pricing_category(
            "embed-100m",
            PricingCallType::Aembedding,
            &TogetherPricingTiers::default()
        ),
        "together-ai-embedding-up-to-150m"
    );
}

#[rstest]
#[case::embedding_of_a_billion_name(PricingCallType::Embedding, "llama-70b")]
#[case::completion_of_a_million_name(PricingCallType::Other, "embed-100m")]
fn the_call_type_picks_the_suffix(#[case] call_type: PricingCallType, #[case] model: &str) {
    assert_eq!(
        pricing_category(model, call_type, &TogetherPricingTiers::default()),
        model
    );
}

#[test]
fn the_tier_limits_come_from_the_injected_tiers() {
    let tiers = TogetherPricingTiers {
        up_to_4b: 2,
        embedding_150m: 50,
        ..TogetherPricingTiers::default()
    };
    assert_eq!(
        pricing_category("llama-3b", PricingCallType::Other, &tiers),
        "together-ai-4.1b-8b"
    );
    assert_eq!(
        pricing_category("embed-100m", PricingCallType::Embedding, &tiers),
        "together-ai-embedding-151m-to-350m"
    );
}

#[test]
fn the_default_tiers_are_litellms_limits() {
    let tiers = TogetherPricingTiers::default();
    assert_eq!(tiers.up_to_4b, 4);
    assert_eq!(tiers.up_to_8b, 8);
    assert_eq!(tiers.up_to_21b, 21);
    assert_eq!(tiers.up_to_41b, 41);
    assert_eq!(tiers.up_to_80b, 80);
    assert_eq!(tiers.up_to_110b, 110);
    assert_eq!(tiers.embedding_150m, 150);
    assert_eq!(tiers.embedding_350m, 350);
}
