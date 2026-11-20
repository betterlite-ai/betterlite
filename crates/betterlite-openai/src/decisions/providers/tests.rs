#![cfg(test)]

use rstest::rstest;

use super::{DecisionsProvider, supported_decisions_providers};

const SUPPORTED: [&str; 8] = [
    "openai",
    "openrouter",
    "azure_ai",
    "perplexity",
    "typesafe",
    "strands_decider",
    "cloudflare",
    "hosted_vllm",
];

#[test]
fn providers_lists_the_eight_supported_values_in_declaration_order() {
    assert_eq!(
        DecisionsProvider::ALL.map(DecisionsProvider::as_str),
        SUPPORTED
    );
    assert_eq!(supported_decisions_providers(), SUPPORTED);
}

#[rstest]
#[case::openai(DecisionsProvider::Openai, "openai")]
#[case::openrouter(DecisionsProvider::Openrouter, "openrouter")]
#[case::azure_ai(DecisionsProvider::AzureAi, "azure_ai")]
#[case::perplexity(DecisionsProvider::Perplexity, "perplexity")]
#[case::typesafe(DecisionsProvider::Typesafe, "typesafe")]
#[case::strands_decider(DecisionsProvider::StrandsDecider, "strands_decider")]
#[case::cloudflare(DecisionsProvider::Cloudflare, "cloudflare")]
#[case::hosted_vllm(DecisionsProvider::HostedVllm, "hosted_vllm")]
fn providers_from_provider_name_round_trips_each_value(
    #[case] provider: DecisionsProvider,
    #[case] name: &str,
) {
    assert_eq!(provider.as_str(), name);
    assert_eq!(DecisionsProvider::from_provider_name(name), Some(provider));
}

#[rstest]
#[case::different_provider("anthropic")]
#[case::wrong_case("OpenAI")]
#[case::empty("")]
#[case::no_decisions_config("openai_like")]
#[case::prefix_of_a_value("azure")]
#[case::strands_without_the_suffix("strands")]
#[case::vllm_without_the_prefix("vllm")]
fn providers_from_provider_name_rejects_other_names(#[case] name: &str) {
    assert_eq!(DecisionsProvider::from_provider_name(name), None);
}
