#![cfg(test)]

use rstest::rstest;

use super::PerplexityDecisionsConfig;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig as _;

fn second_key_only(name: &str) -> String {
    match name {
        "PERPLEXITY_API_KEY" => String::from("second"),
        _ => String::new(),
    }
}

fn both_keys(name: &str) -> String {
    match name {
        "PERPLEXITYAI_API_KEY" => String::from("first"),
        "PERPLEXITY_API_KEY" => String::from("second"),
        "PERPLEXITY_API_BASE" => String::from("https://env.example"),
        _ => String::new(),
    }
}

#[test]
fn the_config_names_the_perplexity_decisions_endpoint() {
    assert_eq!(PerplexityDecisionsConfig::PATH, "/v1/decisions");
    assert_eq!(
        PerplexityDecisionsConfig::API_KEY_ENV,
        ["PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"]
    );
    assert_eq!(
        PerplexityDecisionsConfig::API_BASE_ENV,
        ["PERPLEXITY_API_BASE"]
    );
    assert_eq!(
        PerplexityDecisionsConfig.get_default_api_base(),
        Some(String::from("https://api.perplexity.ai"))
    );
}

#[test]
fn the_other_attributes_keep_the_trait_defaults() {
    let required: bool = PerplexityDecisionsConfig::API_KEY_REQUIRED;
    assert!(required);
    let safety_identifier: bool = PerplexityDecisionsConfig::SUPPORTS_SAFETY_IDENTIFIER;
    assert!(!safety_identifier);
    assert_eq!(PerplexityDecisionsConfig.canonical_model("pplx"), "pplx");
    assert_eq!(
        PerplexityDecisionsConfig.missing_api_base_message("perplexity"),
        "api_base is required for Decisions provider 'perplexity'"
    );
}

#[rstest]
#[case::default_base("https://api.perplexity.ai", "https://api.perplexity.ai/v1/decisions")]
#[case::trailing_v1(
    "https://api.perplexity.ai/v1",
    "https://api.perplexity.ai/v1/decisions"
)]
#[case::trailing_slash("https://proxy.example/v1/", "https://proxy.example/v1/decisions")]
#[case::custom_path(
    "https://proxy.example/custom",
    "https://proxy.example/custom/v1/decisions"
)]
fn the_complete_url_drops_one_v1_before_the_path(#[case] base: &str, #[case] url: &str) {
    assert_eq!(
        PerplexityDecisionsConfig.get_complete_url(base, "pplx"),
        url
    );
}

#[test]
fn the_second_key_name_is_read_when_the_first_is_unset() {
    assert_eq!(
        PerplexityDecisionsConfig.resolve_api_key(None, second_key_only),
        Some(String::from("second"))
    );
    assert_eq!(
        PerplexityDecisionsConfig.resolve_api_base(None, second_key_only),
        Some(String::from("https://api.perplexity.ai"))
    );
}

#[test]
fn the_first_key_name_and_the_base_variable_win() {
    assert_eq!(
        PerplexityDecisionsConfig.resolve_api_key(None, both_keys),
        Some(String::from("first"))
    );
    assert_eq!(
        PerplexityDecisionsConfig.resolve_api_base(None, both_keys),
        Some(String::from("https://env.example"))
    );
}
