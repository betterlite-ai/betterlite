#![cfg(test)]

use betterlite_error::facts::presence::Field;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde::de::DeserializeOwned;
use serde_json::Number;
use thiserror::Error;

use super::config::{
    AuthTokenRequest, BaseGenerateContentConfig, EnvironmentRequest, GenerateContentRequest,
    RequestHeaders, ResponseHeaders, ResponseTransform,
};
use super::records::{
    ContentListUnion, GenerateContentConfigDict, GenerateContentRequestDict,
    GenerateContentResponse, ToolConfigDict,
};

#[derive(Debug, Error)]
enum FixtureError {
    #[error("the fixture config refused the request")]
    Refused,
}

struct Fixture;

impl BaseGenerateContentConfig for Fixture {
    type Error = FixtureError;

    fn supported_optional_params(&self, model: &str) -> Vec<String> {
        vec![model.to_owned()]
    }

    fn map_optional_params(
        &self,
        config: &GenerateContentConfigDict,
        model: &str,
    ) -> GenerateContentConfigDict {
        let mut mapped = config.clone();
        mapped.insert(String::from("model"), Json::Text(model.to_owned()));
        mapped
    }

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<ResponseHeaders, FixtureError> {
        Ok(request.headers.clone())
    }

    fn transform_request(
        &self,
        request: &GenerateContentRequest<'_>,
    ) -> Result<GenerateContentRequestDict, FixtureError> {
        Ok(GenerateContentRequestDict {
            model: request.model.to_owned(),
            contents: request.contents.clone(),
            tools: request
                .tools
                .map_or(Field::Missing, |tools| Field::Value(tools.clone())),
            generation_config: Field::Value(request.generation_config.clone()),
            extra: Object::new(),
        })
    }

    fn transform_response(
        &self,
        request: &ResponseTransform<'_>,
    ) -> Result<GenerateContentResponse, FixtureError> {
        serde_json::from_slice(&request.raw_response.body).map_err(|_source| FixtureError::Refused)
    }

    fn auth_token_and_url(
        &self,
        _request: &AuthTokenRequest<'_>,
    ) -> Result<(RequestHeaders, String), FixtureError> {
        Err(FixtureError::Refused)
    }
}

fn headers(pairs: &[(&str, &str)]) -> ResponseHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn decode<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).expect("the record decodes")
}

// both sides parse into Json, whose object equality ignores key order
fn encode<T>(record: &T) -> Json
where
    T: serde::Serialize,
{
    let text = serde_json::to_string(record).expect("the record encodes");
    serde_json::from_str(&text).expect("the record is JSON")
}

fn fixture(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

#[test]
fn the_top_level_fields_are_the_four_native_names_in_order() {
    assert_eq!(
        Fixture.request_top_level_fields(),
        ["safetySettings", "toolConfig", "cachedContent", "labels"]
    );
}

#[rstest]
#[case::present(
    headers(&[("retry-after", "2")]),
    Some(vec![(String::from("retry-after"), String::from("2"))])
)]
#[case::empty(ResponseHeaders::new(), Some(Vec::new()))]
fn the_error_class_carries_the_exact_status_message_and_headers(
    #[case] sent: ResponseHeaders,
    #[case] expected: Option<Vec<(String, String)>>,
) {
    let error = Fixture.get_error_class("slow down", 429, &sent);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(error.headers, expected);
}

#[test]
fn the_error_class_synthesizes_the_documented_request_and_response() {
    let error = Fixture.get_error_class("", 500, &ResponseHeaders::new());
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "");
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.request.method, "POST");
    assert_eq!(error.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(error.response.status_code, 500);
    assert_eq!(error.body, None);
}

#[test]
fn a_new_response_starts_with_empty_hidden_params() {
    assert!(GenerateContentResponse::default().hidden_params.is_empty());
    let response: GenerateContentResponse = decode(r#"{"candidates": []}"#);
    assert!(response.hidden_params.is_empty());
}

#[test]
fn hidden_params_given_at_construction_are_kept() {
    let response: GenerateContentResponse =
        decode(r#"{"_hidden_params": {"response_cost": "0.01"}, "candidates": []}"#);
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&Json::Text(String::from("0.01")))
    );
    assert!(!response.wire.contains_key("_hidden_params"));
}

#[test]
fn hidden_params_stay_off_the_serialized_response() {
    let mut response: GenerateContentResponse = decode(r#"{"modelVersion": "v1"}"#);
    response.hidden_params.insert(
        String::from("response_cost"),
        Json::Text(String::from("0.01")),
    );
    assert_eq!(encode(&response), fixture(r#"{"modelVersion": "v1"}"#));
}

#[test]
fn the_response_keeps_every_field_the_reply_carries() {
    let reply = r#"{
        "candidates": [{"content": {"parts": [{"text": "hi"}]}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 3, "totalTokenCount": 5},
        "modelVersion": "gemini-2.5-flash",
        "vendorExtension": {"kept": true}
    }"#;
    let response: GenerateContentResponse = decode(reply);
    assert!(response.wire.contains_key("vendorExtension"));
    assert_eq!(encode(&response), fixture(reply));
}

#[test]
fn the_request_dict_keeps_the_declared_fields_and_the_unknown_ones() {
    let body = r#"{
        "model": "gemini-2.5-flash",
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
        "tools": null,
        "generationConfig": {"temperature": 0},
        "systemInstruction": {"parts": [{"text": "behave"}]}
    }"#;
    let request: GenerateContentRequestDict = decode(body);
    assert_eq!(request.model, "gemini-2.5-flash");
    assert_eq!(request.tools, Field::Null);
    assert_eq!(
        request
            .generation_config
            .as_ref()
            .and_then(|config| config.get("temperature")),
        Some(&Json::Number(Number::from(0_i32)))
    );
    assert!(request.extra.contains_key("systemInstruction"));
    assert_eq!(encode(&request), fixture(body));
}

#[test]
fn the_request_dict_omits_absent_optional_fields() {
    let body = r#"{"model": "m", "contents": []}"#;
    let request: GenerateContentRequestDict = decode(body);
    assert!(request.tools.is_missing());
    assert!(request.generation_config.is_missing());
    assert!(request.extra.is_empty());
    assert_eq!(encode(&request), fixture(body));
}

#[test]
fn the_config_dict_keeps_every_key_as_sent() {
    let body = r#"{
        "temperature": 0,
        "stopSequences": [],
        "responseMimeType": "",
        "unknownKey": {"nested": null}
    }"#;
    let config: GenerateContentConfigDict = decode(body);
    assert_eq!(config.len(), 4);
    assert_eq!(encode(&config), fixture(body));
}

#[test]
fn the_content_list_and_tool_config_carry_any_declared_json() {
    let contents: ContentListUnion = decode(r#"["hello", {"role": "user", "parts": []}]"#);
    assert!(matches!(contents, Json::List(_)));
    let tools: ToolConfigDict = decode(r#"[{"functionDeclarations": []}]"#);
    assert!(matches!(tools, Json::List(_)));
}
