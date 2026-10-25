#![cfg(test)]
use super::forwarding::{ForwardingFailure, ResponseForward, forward_response};
use super::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings, is_azure_gpt5_model,
    is_gpt_audio_model, is_gpt_reasoning_series_name, is_o_series_model,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, json};
use std::collections::BTreeSet;
use std::iter::once;
fn catalog() -> CatalogCapabilities {
    let chat_models = ["o3", "gpt-audio", "gpt-4o"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    CatalogCapabilities {
        chat_models,
        text_models: BTreeSet::new(),
    }
}
fn config(family: ConfigFamily) -> OpenAiConfig {
    OpenAiConfig {
        family,
        api_key: Some(SecretString::from("fixture-secret")),
        headers: Headers::from([(String::from("x-test"), String::from("yes"))]),
        settings: OpenAiSettings::default(),
    }
}
#[rstest]
#[case::o_series_routing(0)] #[case::gpt5_routing(1)] #[case::audio_routing(2)] #[case::catalog_user_only(3)] #[case::old_gpt_response_format_drop(4)] #[case::unknown_reasoning_effort(5)] #[case::base_parameter_allowlist(6)] #[case::base_auth_override(7)] #[case::base_projection_forwarding(8)] #[case::base_request_overlay(9)] #[case::openai_error_defaults(10)] #[case::metadata_filter_limit(11)] #[case::overlapping_api_base_path(12)] #[case::azure_gpt5_route_predicate(13)]
#[rustfmt::skip]
fn openai_base_config_cases(#[case] case: u8) { let models = catalog(); match case {
0 => assert!(is_o_series_model("openai/o3", &models) && !is_o_series_model("o4", &models)),
1 => assert!(is_gpt_reasoning_series_name("proxy/gpt-5.1") && !is_gpt_reasoning_series_name("gpt-5-chat")),
2 => assert!(is_gpt_audio_model("gpt-audio", &models) && !is_gpt_audio_model("responses/gpt-audio", &models) && !is_gpt_audio_model("text-audio", &CatalogCapabilities { chat_models: BTreeSet::new(), text_models: once(String::from("text-audio")).collect() })),
3 => assert!(config(ConfigFamily::OpenAi).supported_params("responses/gpt-4o", &models).contains(&"user")),
4 => { let supported = config(ConfigFamily::OpenAi).supported_params("gpt-4", &models); assert!(!supported.contains(&"response_format") && config(ConfigFamily::OpenAi).supported_params("gpt-4o", &models).contains(&"response_format")); },
5 => assert_ne!(config(ConfigFamily::OpenAi).supported_params("custom", &models), config(ConfigFamily::AzureAiStudio).supported_params("custom", &models)),
6 => { let mut existing = Map::from_iter([(String::from("existing"), json!(true))]); config(ConfigFamily::OpenAi).map_params("custom", &Map::from_iter([(String::from("reasoning_effort"), json!("high")), (String::from("unsupported"), json!(true))]), &mut existing, &models); assert!(existing.contains_key("reasoning_effort") && !existing.contains_key("unsupported") && existing.contains_key("existing")); },
7 => assert_eq!(config(ConfigFamily::Other).validate_environment()["Authorization"], "Bearer fixture-secret"),
8 => { let headers = Headers::from([(String::from("x-id"), String::from("abc"))]); let result = forward_response(|raw, _| { assert_eq!(raw, "{}"); Ok::<(), ForwardingFailure>(()) }, |value, _, _| Ok::<_, ForwardingFailure>(value), ResponseForward { raw_text: "{}", raw_headers: &headers, decoded: serde_json::from_str(r#"{"ok":true}"#), hidden_headers: &headers }); result.expect("valid response"); let failure = forward_response(|_, _| Ok::<(), ForwardingFailure>(()), |value, _, _| Ok::<_, ForwardingFailure>(value), ResponseForward { raw_text: "not-json", raw_headers: &headers, decoded: serde_json::from_str("not-json"), hidden_headers: &headers }); failure.expect_err("invalid response"); },
9 => { let body = config(ConfigFamily::Other).request_body("gpt-4o", json!([]), &Map::from_iter([(String::from("temperature"), json!(0.2_f64)), (String::from("model"), json!("override"))])); assert_eq!(body["model"], "override"); assert!(body.contains_key("messages")); let mut configured = config(ConfigFamily::Other); configured.settings.frequency_penalty = Some(json!(1_i32)); assert_eq!(configured.get_config().get("frequency_penalty"), Some(&json!(1_i32))); },
10 => { let record = super::forwarding::OpenAiErrorRecord::from_parts(super::forwarding::OpenAiErrorParts { status_code: 429, message: String::from("busy"), headers: Some(Headers::from([(String::from("x"), String::from("y"))])), request: None, response: None, body: Some(Map::from_iter([(String::from("detail"), json!("body"))])) }); assert_eq!(record.request.url, "https://api.openai.com/v1"); assert!(record.headers.is_some() && record.body.is_some()); },
11 => { let metadata = IndexMap::from_iter(once((String::from("hidden_params"), json!("hidden"))).chain(once((String::from("number"), json!(3_i32))).chain((0_i32..18_i32).map(|index| (format!("key{index:02}"), json!(format!("value{index:02}"))))))); let filtered = super::metadata_filter_limit(Some(&metadata)).expect("metadata"); assert_eq!(filtered.len(), 16); assert!(!filtered.contains_key("hidden_params") && !filtered.contains_key("number") && filtered.contains_key("key15") && !filtered.contains_key("key16")); assert!(super::metadata_filter_limit(None).is_none()); },
12 => assert_eq!(super::overlapping_api_base_path("https://example.test/v1", "/v1/chat").expect("valid URL"), "https://example.test/v1/chat"),
13 => assert!(is_azure_gpt5_model("gpt5_series/custom") && !is_azure_gpt5_model("GPT5_SERIES/custom")),
_ => panic!("unknown case {case}") } }
#[test]
#[rustfmt::skip]
fn openai_base_config_credential_identity_isolation() { let first = config(ConfigFamily::OpenAi); let second = OpenAiConfig { family: ConfigFamily::OpenAi, api_key: Some(SecretString::from("other")), headers: Headers::new(), settings: OpenAiSettings::default() }; assert_ne!(first.validate_environment(), second.validate_environment()); }
#[test]
#[rustfmt::skip]
fn openai_base_config_secret_safe_failure() { let safe = OpenAiConfig { family: ConfigFamily::OpenAi, api_key: Some(SecretString::from("fixture-secret")), headers: Headers::from([(String::from("Authorization"), String::from("Bearer fixture-secret"))]), settings: OpenAiSettings::default() }; let debug = format!("{safe:?}"); assert!(!debug.contains("fixture-secret") && debug.contains("OpenAiConfig")); }
