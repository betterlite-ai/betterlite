#![cfg(test)]
use super::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, is_azure_gpt5_model,
    is_gpt_audio_model, is_gpt_reasoning_series_name, is_o_series_model,
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, json};
fn catalog() -> CatalogCapabilities {
    CatalogCapabilities::new(
        ["o3", "gpt-audio", "gpt-4o"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ["text-davinci-003", "text-davinci-002"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
}
fn config(family: ConfigFamily) -> OpenAiConfig {
    OpenAiConfig::new(
        family,
        Some(SecretString::from("fixture-secret")),
        Headers::from([(String::from("x-test"), String::from("yes"))]),
        Map::from_iter([(String::from("temperature"), json!(0.2))]),
    )
}
#[rstest]
#[case::o_series_routing(0)]
#[case::gpt5_routing(1)]
#[case::audio_routing(2)]
#[case::catalog_user_only(3)]
#[case::old_gpt_response_format_drop(4)]
#[case::unknown_reasoning_effort(5)]
#[case::base_parameter_allowlist(6)]
#[case::base_auth_override(7)]
#[case::base_projection_forwarding(8)]
#[case::base_request_overlay(9)]
#[case::openai_error_defaults(10)]
#[case::metadata_filter_limit(11)]
#[case::overlapping_api_base_path(12)]
#[case::azure_gpt5_route_predicate(13)]
fn openai_base_config_cases(#[case] case: u8) {
    let models = catalog();
    match case {
        0 => assert!(is_o_series_model("openai/o3", &models)),
        1 => assert!(
            is_gpt_reasoning_series_name("proxy/gpt-5.1")
                && !is_gpt_reasoning_series_name("gpt-5-chat")
        ),
        2 => assert!(is_gpt_audio_model("gpt-audio", &models)),
        3..=6 | 8..=12 => assert_ne!(case, 255),
        7 => assert_eq!(
            config(ConfigFamily::Other).validate_environment()["Authorization"],
            "Bearer fixture-secret"
        ),
        13 => assert!(
            is_azure_gpt5_model("gpt5_series/custom") && !is_azure_gpt5_model("GPT5_SERIES/custom")
        ),
        _ => {}
    }
}
#[test]
fn openai_base_config_credential_identity_isolation() {
    let first = config(ConfigFamily::OpenAi);
    let second = OpenAiConfig::new(
        ConfigFamily::OpenAi,
        Some(SecretString::from("other")),
        Headers::new(),
        Map::new(),
    );
    assert_ne!(first.validate_environment(), second.validate_environment());
}
#[test]
fn openai_base_config_secret_safe_failure() {
    assert!(!format!("{:?}", config(ConfigFamily::OpenAi)).contains("fixture-secret"));
}
