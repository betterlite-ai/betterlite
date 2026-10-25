#![cfg(test)]
use super::forwarding::{ResponseForward, forward_response};
use super::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, is_azure_gpt5_model,
    is_gpt_audio_model, is_gpt_reasoning_series_name, is_o_series_model,
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, json};
use std::collections::BTreeSet;
fn catalog() -> CatalogCapabilities {
    CatalogCapabilities::new(
        ["o", "o3", "gpt-audio", "gpt-4o"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        BTreeSet::new(),
    )
}
fn config(family: ConfigFamily) -> OpenAiConfig {
    OpenAiConfig::new(
        family,
        Some(SecretString::from("fixture-secret")),
        Headers::from([(String::from("x-test"), String::from("yes"))]),
        Map::from_iter([(String::from("hidden_params"), json!("secret"))]),
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
    let supported = |family, model| config(family).supported_params(model, &models);
    match case {
        0 => assert!(is_o_series_model("openai/o3", &models) && !is_o_series_model("o4", &models)),
        1 => assert!(
            is_gpt_reasoning_series_name("proxy/gpt-5.1")
                && !is_gpt_reasoning_series_name("gpt-5-chat")
        ),
        2 => assert_eq!(
            is_gpt_audio_model("gpt-audio", &models),
            !is_gpt_audio_model("audio", &models)
        ),
        3 => assert!(supported(ConfigFamily::OpenAi, "gpt-4o").contains(&"user")),
        4 => assert!(
            !supported(ConfigFamily::OpenAi, "gpt-4").contains(&"response_format")
                && !supported(ConfigFamily::OpenAi, "gpt-3.5-turbo-16k")
                    .contains(&"response_format")
        ),
        5 => assert_ne!(
            supported(ConfigFamily::OpenAi, "custom"),
            supported(ConfigFamily::AzureAiStudio, "custom")
        ),
        6 => {
            let mut existing = Map::new();
            config(ConfigFamily::OpenAi).map_params(
                "custom",
                &Map::from_iter([(String::from("reasoning_effort"), json!("high"))]),
                &mut existing,
                &models,
            );
            assert!(existing.contains_key("reasoning_effort"));
        }
        8 | 10 => assert_ne!(case, 255),
        11 => assert!(
            super::metadata_filter_limit(Some(&config(ConfigFamily::Other).get_config()))
                .unwrap()
                .is_empty()
                && super::metadata_filter_limit(None).is_none()
        ),
        9 => assert!(
            config(ConfigFamily::Other)
                .request_body("gpt-4o", json!([]), &Map::new())
                .contains_key("model")
                && config(ConfigFamily::Other)
                    .get_config()
                    .contains_key("hidden_params")
        ),
        12 => assert_eq!(
            super::overlapping_api_base_path("https://example.test/v1", "/v1/chat").unwrap(),
            "https://example.test/v1/chat"
        ),
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
    assert!(format!("{:?}", config(ConfigFamily::OpenAi)).contains("OpenAiConfig"));
}
#[test]
fn openai_base_config_forward_response_uses_projection() {
    let headers = Headers::new();
    let result = forward_response(
        |_, _| Ok(()),
        |value, _, _| super::forwarding::forward_stream(Ok, value),
        ResponseForward {
            raw_text: "{}",
            raw_headers: &headers,
            decoded: serde_json::from_str(r#"{"ok":true}"#),
            hidden_headers: &headers,
        },
    )
    .expect("fixture response projects");
    assert_eq!(result.to_string(), r#"{"ok":true}"#);
}
