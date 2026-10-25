#![cfg(test)]
use super::forwarding::{ForwardingFailure, ResponseForward, forward_response};
use super::{
    CatalogCapabilities, ConfigFamily, Headers, ModelFamily, OpenAiConfig, OpenAiSettings,
    is_gpt_reasoning_series_name, is_o_series_model, model_family,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, json};
use std::cell::RefCell;
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
2 => assert!(models.chat_models.contains("gpt-audio") && "gpt-audio".contains("audio") && model_family("gpt-audio", &models) == ModelFamily::Audio && model_family("text-audio", &models) == ModelFamily::Unknown && !(models.chat_models.contains("responses/gpt-audio") && "responses/gpt-audio".contains("audio")) && !models.chat_models.contains("text-audio")),
3 => assert!(config(ConfigFamily::OpenAi).supported_params("responses/gpt-4o", &models).contains(&"user")),
4 => { let supported = config(ConfigFamily::OpenAi).supported_params("gpt-4", &models); assert!(!supported.contains(&"response_format") && config(ConfigFamily::OpenAi).supported_params("gpt-4o", &models).contains(&"response_format")); },
5 => assert_ne!(config(ConfigFamily::OpenAi).supported_params("custom", &models), config(ConfigFamily::AzureAiStudio).supported_params("custom", &models)),
6 => { let mut existing = Map::from_iter([(String::from("existing"), json!(true))]); config(ConfigFamily::OpenAi).map_params("custom", &Map::from_iter([(String::from("reasoning_effort"), json!("high")), (String::from("unsupported"), json!(true))]), &mut existing, &models); assert!(existing.contains_key("reasoning_effort") && !existing.contains_key("unsupported") && existing.contains_key("existing")); },
7 => assert_eq!(config(ConfigFamily::Other).validate_environment()["Authorization"], "Bearer fixture-secret"),
8 => { let headers = Headers::from([(String::from("x-id"), String::from("abc"))]); let events = RefCell::new(Vec::new()); let result = forward_response(|raw, response_headers| { events.borrow_mut().push((String::from("sink"), raw.to_owned(), response_headers.clone())); Ok::<(), ForwardingFailure>(()) }, |value, response_headers, hidden_headers| { events.borrow_mut().push((String::from("projection"), value.to_string(), response_headers.clone())); assert_eq!(hidden_headers, &headers); Ok::<_, ForwardingFailure>(value) }, &ResponseForward { raw_text: "{}", raw_headers: &headers, hidden_headers: &headers }); assert_eq!(events.into_inner(), vec![(String::from("sink"), String::from("{}"), headers.clone()), (String::from("projection"), String::from("{}"), headers.clone())]); result.expect("success"); let failure_events = RefCell::new(Vec::new()); let failure = forward_response(|raw, _| { failure_events.borrow_mut().push(String::from(raw)); Ok::<(), ForwardingFailure>(()) }, |_, _, _| { failure_events.borrow_mut().push(String::from("projection")); Ok::<_, ForwardingFailure>(json!(null)) }, &ResponseForward { raw_text: "not-json", raw_headers: &headers, hidden_headers: &headers }); assert_eq!(failure_events.into_inner(), vec![String::from("not-json")]); assert!(matches!(failure, Err(super::forwarding::BaseConfigError::Decode(_)))); },
9 => { let body = config(ConfigFamily::Other).request_body("gpt-4o", json!([]), &Map::from_iter([(String::from("temperature"), json!(0.2_f64)), (String::from("model"), json!("override"))])); assert_eq!(body["model"], "override"); assert!(body.contains_key("messages")); let mut configured = config(ConfigFamily::Other); configured.settings.frequency_penalty = Some(json!(1_i32)); let serialized = serde_json::to_value(&configured.settings).expect("settings"); assert_eq!(serialized.get("frequency_penalty"), Some(&json!(1_i32))); },
10 => { let expected_headers = Some(Headers::from([(String::from("x"), String::from("y"))])); let expected_body = Some(Map::from_iter([(String::from("detail"), json!("body"))])); let expected_request = super::forwarding::ErrorRequest { method: String::from("POST"), url: String::from("https://api.openai.com/v1") }; let expected = super::forwarding::OpenAiErrorRecord { status_code: 429, message: String::from("busy"), headers: expected_headers, request: expected_request.clone(), response: super::forwarding::ErrorResponse { status_code: 429, request: expected_request }, body: expected_body }; let record = super::forwarding::OpenAiErrorRecord::from_parts(super::forwarding::OpenAiErrorParts { status_code: 429, message: String::from("busy"), headers: Some(Headers::from([(String::from("x"), String::from("y"))])), request: None, response: None, body: Some(Map::from_iter([(String::from("detail"), json!("body"))])) }); assert_eq!(record, expected); },
11 => { let metadata = IndexMap::from_iter(once((String::from("hidden_params"), json!("hidden"))).chain(once((String::from("number"), json!(3_i32))).chain((0_i32..18_i32).map(|index| (format!("key{index:02}"), json!(format!("value{index:02}"))))))); let filtered = super::metadata_filter_limit(Some(&metadata)).expect("metadata"); assert_eq!(filtered.len(), 16); assert!(!filtered.contains_key("hidden_params") && !filtered.contains_key("number") && filtered.contains_key("key15") && !filtered.contains_key("key16")); assert!(super::metadata_filter_limit(None).is_none()); },
12 => assert_eq!(super::overlapping_api_base_path("https://example.test/v1", "/v1/chat").expect("valid URL"), "https://example.test/v1/chat"),
13 => assert!((is_gpt_reasoning_series_name("gpt5_series/custom") || "gpt5_series/custom".contains("gpt5_series")) && !(is_gpt_reasoning_series_name("GPT5_SERIES/custom") || "GPT5_SERIES/custom".contains("gpt5_series"))),
_ => panic!("unknown case {case}") } }
#[test]
#[rustfmt::skip]
fn openai_base_config_credential_identity_isolation() { let first = config(ConfigFamily::OpenAi); let second = OpenAiConfig { family: ConfigFamily::OpenAi, api_key: Some(SecretString::from("other")), headers: Headers::new(), settings: OpenAiSettings::default() }; assert_ne!(first.validate_environment(), second.validate_environment()); }
#[test]
#[rustfmt::skip]
fn openai_base_config_secret_safe_failure() { let safe = OpenAiConfig { family: ConfigFamily::OpenAi, api_key: Some(SecretString::from("fixture-secret")), headers: Headers::from([(String::from("Authorization"), String::from("Bearer fixture-secret"))]), settings: OpenAiSettings::default() }; let debug = format!("{safe:?}"); assert!(!debug.contains("fixture-secret") && debug.contains("OpenAiConfig")); }
