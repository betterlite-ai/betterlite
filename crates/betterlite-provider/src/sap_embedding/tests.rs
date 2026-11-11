#![cfg(test)]

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;

use super::environment::SapEmbeddingConfig;
use super::error::{SapEmbeddingError, get_error_class};
use super::params::{get_supported_openai_params, map_openai_params};
use super::request::transform_embedding_request;
use super::response::transform_embedding_response;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn transform(optional_params: &str) -> Result<Json, SapEmbeddingError> {
    transform_embedding_request(
        "text-embedding-3-large",
        &EmbeddingInput::Text(String::from("hi")),
        &object(optional_params),
    )
}

#[test]
fn supported_params_include_dimensions_only_for_text_embedding_3() {
    assert_eq!(
        get_supported_openai_params("text-embedding-3-large"),
        vec!["encoding_format", "dimensions"]
    );
    assert_eq!(
        get_supported_openai_params("text-embedding-ada-002"),
        vec!["encoding_format"]
    );
}

#[test]
fn map_openai_params_returns_optional_params_unchanged() {
    let optional = object(r#"{"encoding_format":"float","dimensions":3,"type":"text"}"#);
    let non_default = object(r#"{"encoding_format":"base64","dimensions":5}"#);
    assert_eq!(
        map_openai_params(
            &non_default,
            optional.clone(),
            "text-embedding-3-large",
            true
        ),
        optional
    );
}

#[test]
fn request_builds_the_embedding_body() {
    let body = transform_embedding_request(
        "text-embedding-3-large",
        &EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]),
        &object(
            r#"{
                "version":"v1",
                "parameters":{"x":1},
                "timeout":5,
                "max_retries":2,
                "type":"query",
                "masking":{"providers":[{"type":"sap_data_privacy_integration","method":"anonymization","entities":[{"type":"profile-email"}]}]}
            }"#,
        ),
    )
    .unwrap();
    assert_eq!(
        body,
        json(
            r#"{"config":{"modules":{
                "embeddings":{"model":{
                    "name":"text-embedding-3-large",
                    "version":"v1",
                    "params":{"x":1},
                    "timeout":5,
                    "max_retries":2
                }},
                "masking":{"providers":[{
                    "type":"sap_data_privacy_integration",
                    "method":"anonymization",
                    "entities":[{"type":"profile-email"}]
                }]}
            }},"input":{"text":["a","b"],"type":"query"}}"#
        )
    );
}

#[test]
fn request_defaults_version_and_params() {
    assert_eq!(
        transform("{}").unwrap(),
        json(
            r#"{"config":{"modules":{"embeddings":{"model":{
                "name":"text-embedding-3-large",
                "version":"latest",
                "params":{}
            }}}},"input":{"text":"hi"}}"#
        )
    );
}

#[test]
fn request_treats_null_version_and_parameters_as_their_defaults() {
    assert_eq!(
        transform(r#"{"version":null,"parameters":null}"#).unwrap(),
        json(
            r#"{"config":{"modules":{"embeddings":{"model":{
                "name":"text-embedding-3-large",
                "version":"latest",
                "params":{}
            }}}},"input":{"text":"hi"}}"#
        )
    );
}

#[test]
fn request_ignores_encoding_format_and_dimensions() {
    let body = transform(r#"{"encoding_format":"float","dimensions":3,"type":"text"}"#).unwrap();
    assert_eq!(
        body,
        json(
            r#"{"config":{"modules":{"embeddings":{"model":{
                "name":"text-embedding-3-large",
                "version":"latest",
                "params":{}
            }}}},"input":{"text":"hi","type":"text"}}"#
        )
    );
}

#[test]
fn request_rejects_token_id_inputs() {
    let error =
        transform_embedding_request("m", &EmbeddingInput::Tokens(vec![1, 2]), &Object::new())
            .unwrap_err();
    assert!(matches!(error, SapEmbeddingError::Input));
}

#[test]
fn request_rejects_token_list_inputs() {
    let error = transform_embedding_request(
        "m",
        &EmbeddingInput::TokenLists(vec![vec![1, 2]]),
        &Object::new(),
    )
    .unwrap_err();
    assert!(matches!(error, SapEmbeddingError::Input));
}

#[rstest]
#[case::bogus(r#"{"type":"bogus"}"#)]
#[case::number(r#"{"type":"5"}"#)]
fn request_rejects_unknown_input_types(#[case] optional_params: &str) {
    let error = transform(optional_params).unwrap_err();
    assert!(matches!(error, SapEmbeddingError::InputType));
}

#[rstest]
#[case::timeout_zero(r#"{"timeout":0}"#)]
#[case::timeout_high(r#"{"timeout":601}"#)]
#[case::retries_negative(r#"{"max_retries":-1}"#)]
#[case::retries_high(r#"{"max_retries":6}"#)]
#[case::timeout_string(r#"{"timeout":"5"}"#)]
fn request_rejects_out_of_range_limits(#[case] optional_params: &str) {
    let _error = transform(optional_params).unwrap_err();
}

#[rstest]
#[case::timeout_low(r#"{"timeout":1}"#)]
#[case::timeout_high(r#"{"timeout":600}"#)]
#[case::retries_low(r#"{"max_retries":0}"#)]
#[case::retries_high(r#"{"max_retries":5}"#)]
fn request_accepts_the_limit_edges(#[case] optional_params: &str) {
    let _body = transform(optional_params).unwrap();
}

#[test]
fn request_rejects_non_object_parameters() {
    let error = transform(r#"{"parameters":"x"}"#).unwrap_err();
    assert!(matches!(error, SapEmbeddingError::Parameters));
}

#[test]
fn request_rejects_a_non_string_version() {
    let error = transform(r#"{"version":5}"#).unwrap_err();
    assert!(matches!(error, SapEmbeddingError::Version));
}

#[test]
fn embedding_url_trims_trailing_slashes() {
    let config = SapEmbeddingConfig {
        token: SecretString::from("Bearer case-token"),
        resource_group: String::from("rg1"),
        deployment_url: String::from("https://dep.example.com/v2///"),
    };
    let call = config.prepare().unwrap();
    assert_eq!(call.url, "https://dep.example.com/v2/v2/embeddings");
    assert_eq!(call.headers.len(), 4);
    assert_eq!(
        call.headers.get("authorization").unwrap(),
        "Bearer case-token"
    );
    assert_eq!(call.headers.get("ai-resource-group").unwrap(), "rg1");
    assert_eq!(
        call.headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(call.headers.get("ai-client-type").unwrap(), "LiteLLM");
}

#[test]
fn embedding_response_parses_final_result() {
    let raw = concat!(
        r#"{"final_result":{"object":"list","#,
        r#""data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"#,
        r#""model":"m","usage":{"prompt_tokens":2,"total_tokens":2}}}"#
    );
    let response = transform_embedding_response(raw).unwrap();
    assert_eq!(response.object, "list");
    assert_eq!(response.model, "m");
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.total_tokens, 2);
    assert_eq!(response.data.len(), 1);
}

#[test]
fn embedding_response_without_final_result_fails() {
    let error = transform_embedding_response(r#"{"other":true}"#).unwrap_err();
    assert!(matches!(error, SapEmbeddingError::Response(_)));
}

#[test]
fn error_class_keeps_the_status_and_message_and_ignores_headers() {
    let error = get_error_class("boom", 418, &HeaderMap::new());
    assert_eq!(error.status_code, 418);
    assert_eq!(error.message, "boom");
}
