#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::error::HostedVllmEmbeddingError;
use super::{
    PLACEHOLDER_KEY, SUPPORTED_OPENAI_PARAMS, get_complete_url, get_error_class, map_openai_params,
    transform_embedding_request, transform_embedding_response, validate_environment,
};
use crate::embedding::EmbeddingInput;
use crate::openai_base_config::{Headers, Object};

type Environment = BTreeMap<String, String>;

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("test fixture must be valid JSON")
}

fn environment(api_base: Option<&str>, api_key: Option<&str>) -> Environment {
    let mut values = Environment::new();
    if let Some(base) = api_base {
        values.insert(String::from("HOSTED_VLLM_API_BASE"), String::from(base));
    }
    if let Some(key) = api_key {
        values.insert(String::from("HOSTED_VLLM_API_KEY"), String::from(key));
    }
    values
}

fn header_values(headers: &Headers) -> Value {
    serde_json::to_value(headers).expect("headers must serialize")
}

fn reply(body: &Value) -> Value {
    let raw = serde_json::to_vec(body).expect("the reply must serialize");
    let response =
        transform_embedding_response(&raw, &mut |_text| {}).expect("the reply must decode");
    serde_json::to_value(response).expect("the response must serialize")
}

fn reply_error(body: &Value) -> HostedVllmEmbeddingError {
    let raw = serde_json::to_vec(body).expect("the reply must serialize");
    transform_embedding_response(&raw, &mut |_text| {}).expect_err("the reply must not decode")
}

fn full_reply() -> Value {
    fixture(
        r#"{
            "object": "list",
            "model": "my-model",
            "data": [{"object": "embedding", "embedding": [0.5, 0.25], "index": 0}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 0, "total_tokens": 2}
        }"#,
    )
}

fn provider_parts(error: HostedVllmEmbeddingError) -> (String, u16) {
    match error {
        HostedVllmEmbeddingError::Provider {
            message,
            status_code,
            ..
        } => (message, status_code),
        HostedVllmEmbeddingError::MissingApiBase | HostedVllmEmbeddingError::Shape { .. } => {
            panic!("unexpected error without a provider status");
        }
    }
}

fn params(value: Value) -> Object {
    let Value::Object(fields) = value else {
        panic!("test parameters must be an object");
    };
    fields
}

#[test]
fn headers_add_content_type_and_bearer() {
    let key = SecretString::from(String::from("secret"));
    let headers = validate_environment(&Headers::new(), Some(&key), &environment(None, None));
    assert_eq!(
        header_values(&headers),
        fixture(r#"{"Content-Type": "application/json", "Authorization": "Bearer secret"}"#)
    );
}

#[test]
fn headers_skip_authorization_for_the_placeholder_key() {
    let key = SecretString::from(String::from(PLACEHOLDER_KEY));
    let headers = validate_environment(&Headers::new(), Some(&key), &environment(None, None));
    assert_eq!(
        header_values(&headers),
        fixture(r#"{"Content-Type": "application/json"}"#)
    );
}

#[test]
fn headers_skip_authorization_for_an_empty_argument_key() {
    let key = SecretString::from(String::new());
    let headers =
        validate_environment(&Headers::new(), Some(&key), &environment(None, Some("env")));
    assert_eq!(
        header_values(&headers),
        fixture(r#"{"Content-Type": "application/json"}"#)
    );
}

#[test]
fn headers_read_the_environment_key_when_the_argument_is_missing() {
    let headers = validate_environment(&Headers::new(), None, &environment(None, Some("env-key")));
    assert_eq!(
        header_values(&headers),
        fixture(r#"{"Content-Type": "application/json", "Authorization": "Bearer env-key"}"#)
    );
}

#[test]
fn headers_default_the_key_when_nothing_is_set() {
    let headers = validate_environment(&Headers::new(), None, &environment(None, None));
    assert_eq!(
        header_values(&headers),
        fixture(r#"{"Content-Type": "application/json"}"#)
    );
}

#[test]
fn headers_ignore_an_empty_environment_key() {
    let headers = validate_environment(&Headers::new(), None, &environment(None, Some("")));
    assert_eq!(
        header_values(&headers),
        fixture(r#"{"Content-Type": "application/json"}"#)
    );
}

#[test]
fn caller_headers_win_on_exact_keys() {
    let headers = Headers::from([
        (String::from("Content-Type"), String::from("text/plain")),
        (String::from("Authorization"), String::from("Bearer caller")),
    ]);
    let key = SecretString::from(String::from("secret"));
    let merged = validate_environment(&headers, Some(&key), &environment(None, None));
    assert_eq!(
        header_values(&merged),
        fixture(r#"{"Content-Type": "text/plain", "Authorization": "Bearer caller"}"#)
    );
}

#[test]
fn a_differently_cased_caller_header_is_a_second_entry() {
    let headers = Headers::from([(String::from("content-type"), String::from("text/plain"))]);
    let merged = validate_environment(&headers, None, &environment(None, None));
    assert_eq!(
        header_values(&merged),
        fixture(r#"{"Content-Type": "application/json", "content-type": "text/plain"}"#)
    );
}

#[test]
fn complete_url_appends_embeddings() {
    let url = get_complete_url(Some("http://host/v1"), &environment(None, None));
    assert_eq!(url.expect("url"), "http://host/v1/embeddings");
}

#[test]
fn complete_url_strips_trailing_slashes() {
    let url = get_complete_url(Some("http://host/v1///"), &environment(None, None));
    assert_eq!(url.expect("url"), "http://host/v1/embeddings");
}

#[test]
fn complete_url_keeps_an_existing_embeddings_suffix() {
    let url = get_complete_url(Some("http://host/v1/embeddings/"), &environment(None, None));
    assert_eq!(url.expect("url"), "http://host/v1/embeddings");
}

#[test]
fn complete_url_uses_an_empty_explicit_base_as_given() {
    let url = get_complete_url(Some(""), &environment(Some("http://env"), None));
    assert_eq!(url.expect("url"), "/embeddings");
}

#[test]
fn complete_url_reads_the_environment_base() {
    let url = get_complete_url(None, &environment(Some("http://env/"), None));
    assert_eq!(url.expect("url"), "http://env/embeddings");
}

#[test]
fn complete_url_without_any_base_is_an_error() {
    let error = get_complete_url(None, &environment(None, None)).expect_err("url must fail");
    assert!(matches!(error, HostedVllmEmbeddingError::MissingApiBase));
    assert_eq!(
        error.to_string(),
        "api_base is required for hosted_vllm embeddings"
    );
}

#[test]
fn complete_url_treats_an_empty_environment_base_as_unset() {
    let error = get_complete_url(None, &environment(Some(""), None)).expect_err("url must fail");
    assert!(matches!(error, HostedVllmEmbeddingError::MissingApiBase));
}

#[rstest]
#[case(r#"{"timeout": 5}"#)]
#[case(r#"{"dimensions": 2}"#)]
#[case(r#"{"encoding_format": "float"}"#)]
#[case(r#"{"user": "me"}"#)]
fn map_copies_each_supported_parameter(#[case] given: &str) {
    let expected = fixture(given);
    let mut optional = Object::new();
    map_openai_params(&params(expected.clone()), &mut optional);
    assert_eq!(Value::Object(optional), expected);
}

#[test]
fn map_drops_unsupported_parameters() {
    let mut optional = Object::new();
    map_openai_params(
        &params(fixture(
            r#"{"timeout": 5, "input_type": "search_document"}"#,
        )),
        &mut optional,
    );
    assert_eq!(optional, params(fixture(r#"{"timeout": 5}"#)));
}

#[test]
fn supported_params_are_the_four_litellm_names() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["timeout", "dimensions", "encoding_format", "user"]
    );
}

#[rstest]
#[case(EmbeddingInput::Text(String::from("hi")), r#"["hi"]"#)]
#[case(
    EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]),
    r#"["a", "b"]"#
)]
#[case(EmbeddingInput::Tokens(vec![1, 2]), "[1, 2]")]
#[case(
    EmbeddingInput::TokenLists(vec![vec![1, 2], vec![3]]),
    "[[1, 2], [3]]"
)]
fn requests_always_send_an_input_array(#[case] input: EmbeddingInput, #[case] expected: &str) {
    let body = transform_embedding_request("my-model", &input, &Object::new());
    assert_eq!(body.get("input"), Some(&fixture(expected)));
}

#[test]
fn requests_strip_one_model_prefix() {
    let body = transform_embedding_request(
        "hosted_vllm/hosted_vllm/my-model",
        &EmbeddingInput::Text(String::from("hi")),
        &Object::new(),
    );
    assert_eq!(body.get("model"), Some(&json!("hosted_vllm/my-model")));
}

#[test]
fn requests_append_the_optional_params_after_model_and_input() {
    let body = transform_embedding_request(
        "my-model",
        &EmbeddingInput::Text(String::from("hi")),
        &params(fixture(r#"{"dimensions": 2, "encoding_format": "base64"}"#)),
    );
    assert_eq!(
        Value::Object(body),
        fixture(
            r#"{
                "model": "my-model",
                "input": ["hi"],
                "dimensions": 2,
                "encoding_format": "base64"
            }"#
        )
    );
}

#[test]
fn an_optional_param_can_override_the_input() {
    let body = transform_embedding_request(
        "my-model",
        &EmbeddingInput::Text(String::from("hi")),
        &params(fixture(r#"{"input": "overridden"}"#)),
    );
    assert_eq!(body.get("input"), Some(&json!("overridden")));
}

#[test]
fn a_full_reply_decodes_unchanged() {
    assert_eq!(
        reply(&full_reply()),
        fixture(
            r#"{
                "object": "list",
                "model": "my-model",
                "data": [{"object": "embedding", "embedding": [0.5, 0.25], "index": 0}],
                "usage": {
                    "prompt_tokens": 2,
                    "completion_tokens": 0,
                    "completion_tokens_details": null,
                    "prompt_tokens_details": null,
                    "total_tokens": 2
                }
            }"#
        )
    );
}

#[test]
fn a_reply_without_object_defaults_it_to_list() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .remove("object")
        .expect("object key");
    assert_eq!(reply(&body).get("object"), Some(&json!("list")));
}

#[test]
fn a_reply_keeps_its_object_value() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("object"), json!("embedding_list"));
    assert_eq!(reply(&body).get("object"), Some(&json!("embedding_list")));
}

#[test]
fn a_reply_without_usage_defaults_to_zero_counts() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .remove("usage")
        .expect("usage key");
    assert_eq!(
        reply(&body).get("usage"),
        Some(&fixture(
            r#"{
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "completion_tokens_details": null,
                "prompt_tokens_details": null,
                "total_tokens": 0
            }"#
        ))
    );
}

#[test]
fn a_null_usage_defaults_to_zero_counts() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("usage"), Value::Null);
    assert_eq!(
        reply(&body).get("usage"),
        Some(&fixture(
            r#"{
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "completion_tokens_details": null,
                "prompt_tokens_details": null,
                "total_tokens": 0
            }"#
        ))
    );
}

#[test]
fn usage_missing_counters_default_to_zero() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("usage"), fixture(r#"{"prompt_tokens": 3}"#));
    assert_eq!(
        reply(&body).get("usage"),
        Some(&fixture(
            r#"{
                "prompt_tokens": 3,
                "completion_tokens": 0,
                "completion_tokens_details": null,
                "prompt_tokens_details": null,
                "total_tokens": 0
            }"#
        ))
    );
}

#[test]
fn usage_extra_fields_are_ignored() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("usage"),
        fixture(r#"{"prompt_tokens": 1, "total_tokens": 1, "extra": 9}"#),
    );
    assert_eq!(
        reply(&body).get("usage"),
        Some(&fixture(
            r#"{
                "prompt_tokens": 1,
                "completion_tokens": 0,
                "completion_tokens_details": null,
                "prompt_tokens_details": null,
                "total_tokens": 1
            }"#
        ))
    );
}

#[test]
fn a_missing_model_is_an_error() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .remove("model")
        .expect("model key");
    assert!(matches!(
        reply_error(&body),
        HostedVllmEmbeddingError::Shape { .. }
    ));
}

#[test]
fn a_missing_data_is_an_error() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .remove("data")
        .expect("data key");
    assert!(matches!(
        reply_error(&body),
        HostedVllmEmbeddingError::Shape { .. }
    ));
}

#[test]
fn a_null_data_is_an_error() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("data"), Value::Null);
    assert!(matches!(
        reply_error(&body),
        HostedVllmEmbeddingError::Shape { .. }
    ));
}

#[test]
fn a_data_item_with_a_string_index_is_an_error() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("data"),
        fixture(r#"[{"object": "embedding", "embedding": [0.5], "index": "zero"}]"#),
    );
    assert!(matches!(
        reply_error(&body),
        HostedVllmEmbeddingError::Shape { .. }
    ));
}

#[test]
fn a_non_object_reply_is_an_error() {
    assert!(matches!(
        reply_error(&fixture("[1, 2]")),
        HostedVllmEmbeddingError::Shape { .. }
    ));
}

#[test]
fn a_reply_that_is_not_json_is_an_error() {
    let error = transform_embedding_response(b"{", &mut |_text| {}).expect_err("reply must fail");
    assert!(matches!(error, HostedVllmEmbeddingError::Shape { .. }));
}

#[test]
fn the_post_call_hook_sees_the_reply_text() {
    let raw = br#"{"object": "list", "model": "m", "data": []}"#;
    let expected = r#"{"object": "list", "model": "m", "data": []}"#;
    let mut seen = String::new();
    transform_embedding_response(raw, &mut |text| seen.push_str(text)).expect("reply");
    assert_eq!(seen, expected);
}

#[test]
fn a_reply_with_an_empty_data_array_decodes() {
    let body = fixture(r#"{"model": "m", "data": []}"#);
    assert_eq!(
        reply(&body),
        fixture(
            r#"{
                "object": "list",
                "data": [],
                "model": "m",
                "usage": {
                    "prompt_tokens": 0,
                    "completion_tokens": 0,
                    "completion_tokens_details": null,
                    "prompt_tokens_details": null,
                    "total_tokens": 0
                }
            }"#
        )
    );
}

#[test]
fn the_error_class_keeps_the_message_status_and_headers() {
    let headers = Headers::from([(String::from("x"), String::from("y"))]);
    let error = get_error_class("bad", 429, &headers);
    let HostedVllmEmbeddingError::Provider {
        message,
        status_code,
        headers: kept,
    } = &error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(message, "bad");
    assert_eq!(*status_code, 429);
    assert_eq!(kept, &headers);
    assert_eq!(error.status_code(), 429);
    assert_eq!(error.to_string(), "bad");
}

#[test]
fn non_provider_errors_report_status_500() {
    let missing = HostedVllmEmbeddingError::MissingApiBase;
    assert_eq!(missing.status_code(), 500);
    let shape = transform_embedding_response(b"{}", &mut |_text| {}).expect_err("reply must fail");
    assert_eq!(shape.status_code(), 500);
}

#[test]
fn a_meaningful_error_in_a_full_reply_is_a_provider_error() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(r#"{"message": "boom", "code": 400}"#),
    );
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("boom"), 400)
    );
}

#[test]
fn an_error_only_reply_reports_status_422() {
    assert_eq!(
        provider_parts(reply_error(&fixture(r#"{"error": {"message": "boom"}}"#))),
        (String::from("boom"), 422)
    );
}

#[test]
fn an_error_message_alone_is_meaningful_with_no_code() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), fixture(r#"{"message": "boom"}"#));
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("boom"), 422)
    );
}

#[test]
fn a_blank_error_message_with_a_code_is_meaningful() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(r#"{"message": "", "code": 429}"#),
    );
    assert_eq!(provider_parts(reply_error(&body)), (String::new(), 429));
}

#[test]
fn an_error_code_alone_uses_the_default_message() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), fixture(r#"{"code": 429}"#));
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("Error in response object"), 429)
    );
}

#[test]
fn a_string_error_is_meaningful_with_status_422() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), json!("boom"));
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("Error in response object"), 422)
    );
}

#[test]
fn a_non_integer_error_code_reports_status_422() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(r#"{"message": "boom", "code": "rate_limit"}"#),
    );
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("boom"), 422)
    );
}

#[test]
fn a_null_error_message_with_a_code_uses_the_default_message() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(r#"{"message": null, "code": 429}"#),
    );
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("Error in response object"), 429)
    );
}

#[test]
fn a_large_error_code_reports_status_422() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), fixture(r#"{"code": 70000}"#));
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("Error in response object"), 422)
    );
}

#[test]
fn a_structured_error_message_is_encoded_as_json() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(r#"{"message": {"reason": "boom"}}"#),
    );
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from(r#"{"reason":"boom"}"#), 422)
    );
}

#[test]
fn a_truthy_error_message_that_is_not_a_string_is_meaningful() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), fixture(r#"{"message": true}"#));
    assert_eq!(
        provider_parts(reply_error(&body)),
        (String::from("true"), 422)
    );
}

#[test]
fn a_nonzero_number_error_message_is_meaningful() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), fixture(r#"{"message": 5}"#));
    assert_eq!(provider_parts(reply_error(&body)), (String::from("5"), 422));
}

#[test]
fn a_null_error_in_a_reply_is_ignored() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), Value::Null);
    assert_eq!(reply(&body).get("model"), Some(&json!("my-model")));
}

#[test]
fn an_empty_string_error_is_ignored() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), json!(""));
    assert_eq!(reply(&body).get("model"), Some(&json!("my-model")));
}

#[test]
fn an_empty_error_object_is_ignored() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), fixture("{}"));
    assert_eq!(reply(&body).get("model"), Some(&json!("my-model")));
}

#[test]
fn a_null_error_message_with_a_null_code_is_ignored() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(r#"{"message": null, "code": null}"#),
    );
    assert_eq!(reply(&body).get("model"), Some(&json!("my-model")));
}

#[test]
fn an_empty_error_message_with_a_null_code_is_ignored() {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(r#"{"message": "", "code": null}"#),
    );
    assert_eq!(reply(&body).get("model"), Some(&json!("my-model")));
}

#[test]
fn a_zero_error_message_with_a_null_code_is_ignored() {
    let mut body = full_reply();
    body.as_object_mut()
        .expect("object")
        .insert(String::from("error"), fixture(r#"{"message": 0}"#));
    assert_eq!(reply(&body).get("model"), Some(&json!("my-model")));
}

#[rstest]
#[case::empty_list("[]")]
#[case::empty_object("{}")]
#[case::false_flag("false")]
fn an_empty_container_or_false_error_message_is_ignored(#[case] message: &str) {
    let mut body = full_reply();
    body.as_object_mut().expect("object").insert(
        String::from("error"),
        fixture(&format!(r#"{{"message": {message}}}"#)),
    );
    assert_eq!(reply(&body).get("model"), Some(&json!("my-model")));
}

#[rstest]
fn replays_the_hosted_vllm_embedding_reply_corpus(
    #[files("fuzz/corpus/hosted_vllm_embedding_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _reply = transform_embedding_response(&bytes, &mut |_text| {});
}
