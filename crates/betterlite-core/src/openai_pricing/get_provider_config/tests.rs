#![cfg(test)]

use betterlite_openai::openai_base_config::ConfigFamily;
use rstest::rstest;

use super::get_provider_config;

#[rstest]
#[case::default_model("gpt-4o")]
#[case::empty("")]
#[case::unknown("not-a-known-model")]
#[case::azure("azure/gpt-4o")]
#[case::responses_prefix("responses/gpt-5")]
fn ignores_model(#[case] model: &str) {
    let config = get_provider_config(model);
    assert_eq!(config.family, ConfigFamily::OpenAi);
    assert!(config.api_key.is_none());
    assert!(config.headers.is_empty());
    assert_eq!(serde_json::to_string(&config.settings).unwrap(), "{}");
}
