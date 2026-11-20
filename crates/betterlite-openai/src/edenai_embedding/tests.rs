#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{
    EdenAiEmbeddingError, SUPPORTED_PARAMS, TransformInput, map_openai_params, plan_embedding,
    request_body, transform_response,
};
use crate::edenai_common::{EDENAI_API_BASE, EdenAiEnv, RESPONSE_COST_HEADER};
use crate::embedding::{EmbeddingInput, EmbeddingVector};
use crate::openai_base_config::{Headers, Object};

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn environment(api_key: Option<&str>, api_base: Option<&str>) -> EdenAiEnv {
    EdenAiEnv {
        api_key: api_key.map(key),
        api_base: api_base.map(str::to_owned),
        global_api_key: None,
    }
}

fn caller_headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(value: Value) -> Object {
    value.as_object().unwrap().clone()
}

#[test]
fn supported_params_are_the_three_eden_keys() {
    assert_eq!(SUPPORTED_PARAMS, ["dimensions", "encoding_format", "user"]);
}

#[test]
fn map_openai_params_overlays_only_the_supported_keys() {
    let optional = object(json!({"user": "alice", "model": "kept"}));
    let non_default = object(json!({
        "dimensions": 8,
        "encoding_format": "base64",
        "user": "bob",
        "unknown": 1
    }));
    let mapped = map_openai_params(&non_default, &optional);
    assert_eq!(
        mapped,
        object(json!({
            "user": "bob",
            "model": "kept",
            "dimensions": 8,
            "encoding_format": "base64"
        }))
    );
}

#[test]
fn map_openai_params_without_non_defaults_keeps_the_optional_params() {
    let optional = object(json!({"user": "alice"}));
    assert_eq!(
        map_openai_params(&Object::new(), &optional),
        object(json!({"user": "alice"}))
    );
}

#[rstest]
#[case::text(EmbeddingInput::Text(String::from("hi")), json!("hi"))]
#[case::texts(EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]), json!(["a", "b"]))]
#[case::tokens(EmbeddingInput::Tokens(vec![1, 2]), json!([1, 2]))]
#[case::token_lists(EmbeddingInput::TokenLists(vec![vec![1], vec![2, 3]]), json!([[1], [2, 3]]))]
fn request_body_serializes_every_declared_input_shape(
    #[case] input: EmbeddingInput,
    #[case] expected: Value,
) {
    let body = request_body("m", &input, &Object::new()).unwrap();
    assert_eq!(body.get("model"), Some(&json!("m")));
    assert_eq!(body.get("input"), Some(&expected));
    assert_eq!(body.len(), 2);
}

#[test]
fn request_body_lets_optional_params_overwrite_model_and_input() {
    let optional = object(json!({"model": "other", "input": "other", "user": "alice"}));
    let body = request_body("m", &EmbeddingInput::Text(String::from("hi")), &optional).unwrap();
    assert_eq!(body.get("model"), Some(&json!("other")));
    assert_eq!(body.get("input"), Some(&json!("other")));
    assert_eq!(body.get("user"), Some(&json!("alice")));
}

#[test]
fn plan_embedding_builds_the_url_headers_and_body() {
    let env = environment(Some("environment"), None);
    let headers = caller_headers(&[("x-trace", "1")]);
    let optional = Object::new();
    let input = EmbeddingInput::Text(String::from("hi"));
    let plan = plan_embedding(&TransformInput {
        model: "m",
        input: &input,
        optional_params: &optional,
        headers: &headers,
        api_key: Some(&key("explicit")),
        api_base: Some("https://proxy.example/v3/"),
        env: &env,
    })
    .unwrap();
    assert_eq!(plan.url, "https://proxy.example/v3/embeddings");
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer explicit")
    );
    assert_eq!(
        plan.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(plan.headers.get("x-trace").map(String::as_str), Some("1"));
    assert_eq!(plan.body.get("model"), Some(&json!("m")));
    assert_eq!(plan.body.get("input"), Some(&json!("hi")));
}

#[rstest]
#[case::added(&[], "application/json")]
#[case::replaced(&[("Content-Type", "text/plain")], "application/json")]
#[case::other_case_replaced(&[("content-type", "text/plain")], "application/json")]
fn plan_embedding_replaces_a_caller_content_type(
    #[case] supplied: &[(&str, &str)],
    #[case] expected: &str,
) {
    let env = environment(Some("environment"), None);
    let headers = caller_headers(supplied);
    let optional = Object::new();
    let input = EmbeddingInput::Text(String::from("hi"));
    let plan = plan_embedding(&TransformInput {
        model: "m",
        input: &input,
        optional_params: &optional,
        headers: &headers,
        api_key: Some(&key("explicit")),
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(
        plan.headers.get("Content-Type").map(String::as_str),
        Some(expected)
    );
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer explicit")
    );
    assert_eq!(
        plan.headers
            .keys()
            .filter(|name| name.eq_ignore_ascii_case("content-type"))
            .count(),
        1
    );
}

#[test]
fn plan_embedding_reads_the_environment_key_and_base() {
    let env = environment(Some("environment"), Some("https://gateway.example/v3"));
    let headers = Headers::new();
    let optional = Object::new();
    let input = EmbeddingInput::Text(String::from("hi"));
    let plan = plan_embedding(&TransformInput {
        model: "m",
        input: &input,
        optional_params: &optional,
        headers: &headers,
        api_key: None,
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(plan.url, "https://gateway.example/v3/embeddings");
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer environment")
    );
}

#[test]
fn plan_embedding_uses_the_default_base_without_any_base() {
    let env = environment(Some("key"), None);
    let headers = Headers::new();
    let optional = Object::new();
    let input = EmbeddingInput::Text(String::from("hi"));
    let plan = plan_embedding(&TransformInput {
        model: "m",
        input: &input,
        optional_params: &optional,
        headers: &headers,
        api_key: None,
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(plan.url, format!("{EDENAI_API_BASE}/embeddings"));
}

#[test]
fn plan_embedding_without_a_key_is_the_missing_key_error() {
    let env = environment(None, None);
    let headers = Headers::new();
    let optional = Object::new();
    let input = EmbeddingInput::Text(String::from("hi"));
    let error = plan_embedding(&TransformInput {
        model: "m",
        input: &input,
        optional_params: &optional,
        headers: &headers,
        api_key: None,
        api_base: None,
        env: &env,
    })
    .unwrap_err();
    assert!(matches!(&error, EdenAiEmbeddingError::MissingKey(_)));
    assert_eq!(error.status_code(), 401);
}

const BODY: &str = r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2},"cost":0.5}"#;

#[test]
fn transform_response_reads_the_embedding_and_lifts_the_cost() {
    let reply = transform_response(BODY.as_bytes()).unwrap();
    let response = serde_json::to_value(&reply.response).unwrap();
    assert_eq!(response["object"], json!("list"));
    assert_eq!(response["model"], json!("text-embedding-3-small"));
    assert_eq!(response["data"][0]["embedding"], json!([0.1, 0.2]));
    assert_eq!(response["data"][0]["index"], json!(0));
    assert_eq!(response["usage"]["prompt_tokens"], json!(2));
    assert_eq!(response["usage"]["completion_tokens"], json!(0));
    assert_eq!(response["usage"]["total_tokens"], json!(2));
    let additional =
        serde_json::to_value(reply.hidden_params.get("additional_headers").unwrap()).unwrap();
    assert_eq!(additional[RESPONSE_COST_HEADER], json!(0.5));
}

#[test]
fn transform_response_without_a_cost_leaves_hidden_params_empty() {
    let body = br#"{"object":"list","data":[],"model":"m"}"#;
    let reply = transform_response(body).unwrap();
    assert!(reply.hidden_params.is_empty());
}

#[test]
fn transform_response_reads_the_cost_through_the_header_alias() {
    let body = br#"{"object":"list","data":[],"model":"m","x-edenai-cost":"2"}"#;
    let reply = transform_response(body).unwrap();
    let additional =
        serde_json::to_value(reply.hidden_params.get("additional_headers").unwrap()).unwrap();
    assert_eq!(additional[RESPONSE_COST_HEADER], json!(2));
}

#[test]
fn transform_response_applies_the_embedding_response_defaults() {
    let body = br#"{"data":[]}"#;
    let reply = transform_response(body).unwrap();
    let response = serde_json::to_value(&reply.response).unwrap();
    assert_eq!(response["object"], json!("list"));
    assert_eq!(response["model"], json!(""));
    assert_eq!(response["usage"]["prompt_tokens"], json!(0));
    assert_eq!(response["usage"]["completion_tokens"], json!(0));
    assert_eq!(response["usage"]["total_tokens"], json!(0));
}

#[test]
fn transform_response_fills_a_missing_usage_count_with_zero() {
    let body = br#"{"data":[],"usage":{"prompt_tokens":4,"total_tokens":4}}"#;
    let reply = transform_response(body).unwrap();
    assert_eq!(reply.response.usage.prompt_tokens, 4);
    assert_eq!(reply.response.usage.completion_tokens, 0);
    assert_eq!(reply.response.usage.total_tokens, 4);
}

#[test]
fn transform_response_ignores_a_usage_object_beside_the_counts() {
    let body = br#"{"data":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"cost":9}}"#;
    let reply = transform_response(body).unwrap();
    assert_eq!(reply.response.usage.total_tokens, 3);
    assert!(reply.hidden_params.is_empty());
}

#[test]
fn transform_response_keeps_a_base64_embedding() {
    let body = br#"{"data":[{"object":"embedding","embedding":"AAAA","index":0}]}"#;
    let reply = transform_response(body).unwrap();
    let EmbeddingVector::Base64(text) = &reply.response.data[0].embedding else {
        panic!("the embedding stays base64");
    };
    assert_eq!(text, "AAAA");
}

#[test]
fn transform_response_rejects_a_body_without_data() {
    let error = transform_response(br#"{"object":"list"}"#).unwrap_err();
    assert!(matches!(&error, EdenAiEmbeddingError::Reply { .. }));
    assert_eq!(error.status_code(), 500);
    assert!(error.to_string().contains("embeddings response"), "{error}");
}

#[test]
fn transform_response_rejects_bytes_that_are_not_json() {
    let error = transform_response(b"not json").unwrap_err();
    assert!(matches!(&error, EdenAiEmbeddingError::Reply { .. }));
    assert_eq!(error.status_code(), 500);
}

#[rstest]
#[case::numeric_code(
    br#"{"data":[{"object":"embedding","embedding":[0.1],"index":0}],"error":{"message":"rate limited","code":429}}"#,
    429,
    "rate limited"
)]
#[case::missing_code(br#"{"data":[],"error":{"message":"bad model"}}"#, 422, "bad model")]
#[case::string_error(
    br#"{"data":[],"error":"rate limited"}"#,
    422,
    "Error in response object"
)]
#[case::unrepresentable_code(br#"{"data":[],"error":{"message":"bad","code":"429"}}"#, 422, "bad")]
#[case::negative_code(br#"{"data":[],"error":{"message":"bad","code":-1}}"#, 422, "bad")]
#[case::fractional_code(br#"{"data":[],"error":{"message":"bad","code":429.5}}"#, 422, "bad")]
#[case::out_of_range_code(br#"{"data":[],"error":{"message":"bad","code":70000}}"#, 422, "bad")]
#[case::missing_message(br#"{"error":{"code":500}}"#, 500, "Error in response object")]
fn transform_response_raises_a_meaningful_error_object(
    #[case] body: &[u8],
    #[case] status: u16,
    #[case] message: &str,
) {
    let error = transform_response(body).unwrap_err();
    let EdenAiEmbeddingError::ReplyError {
        status_code,
        message: text,
    } = &error
    else {
        panic!("a meaningful error object is the reply error");
    };
    assert_eq!((*status_code, text.as_str()), (status, message));
    assert_eq!(error.status_code(), status);
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::empty_object(br#"{"data":[],"error":{}}"#)]
#[case::empty_string(br#"{"data":[],"error":""}"#)]
#[case::null(br#"{"data":[],"error":null}"#)]
fn transform_response_ignores_an_empty_error_object(#[case] body: &[u8]) {
    let reply = transform_response(body).unwrap();
    assert!(reply.response.data.is_empty());
}
