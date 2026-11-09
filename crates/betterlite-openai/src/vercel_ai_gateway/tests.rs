#![cfg(test)]

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::VercelAiGatewayEnvironment;
use super::chat::VercelAiGatewayConfig;
use super::embedding::{
    decode_embedding_response, get_complete_url, map_openai_params as map_embedding_params,
    transform_embedding_request, transform_embedding_response, validate_environment,
};
use super::error::get_error_class;
use super::models::{models_url, parse_models};
use super::models_error::ModelsError;
use super::provider_info::provider_info;
use crate::embedding::{EmbeddingError, EmbeddingInput};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use std::error::Error as _;

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("object")
}

fn catalog(chat: &[&str], text: &[&str]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: chat.iter().map(|name| (*name).to_owned()).collect(),
        text_models: text.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn config() -> VercelAiGatewayConfig {
    VercelAiGatewayConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        },
    }
}

fn environment(
    base: Option<&str>,
    key: Option<&str>,
    oidc: Option<&str>,
) -> VercelAiGatewayEnvironment {
    VercelAiGatewayEnvironment {
        vercel_ai_gateway_api_base: base.map(str::to_owned),
        vercel_ai_gateway_api_key: key.map(secret),
        vercel_oidc_token: oidc.map(secret),
    }
}

struct ProviderCase {
    api_base: Option<&'static str>,
    api_key: Option<&'static str>,
    env_base: Option<&'static str>,
    env_key: Option<&'static str>,
    oidc: Option<&'static str>,
    expected_base: &'static str,
    expected_key: Option<&'static str>,
}

#[rstest]
#[case::explicit_wins(ProviderCase { api_base: Some("explicit-base"), api_key: Some("explicit-key"), env_base: Some("env-base"), env_key: Some("env-key"), oidc: Some("oidc"), expected_base: "explicit-base", expected_key: Some("explicit-key") })]
#[case::environment_wins(ProviderCase { api_base: None, api_key: None, env_base: Some("env-base"), env_key: Some("env-key"), oidc: Some("oidc"), expected_base: "env-base", expected_key: Some("env-key") })]
#[case::oidc_token_wins(ProviderCase { api_base: None, api_key: None, env_base: None, env_key: None, oidc: Some("oidc"), expected_base: "https://ai-gateway.vercel.sh/v1", expected_key: Some("oidc") })]
#[case::nothing_set(ProviderCase { api_base: None, api_key: None, env_base: None, env_key: None, oidc: None, expected_base: "https://ai-gateway.vercel.sh/v1", expected_key: None })]
#[case::empty_explicit_falls_through(ProviderCase { api_base: Some(""), api_key: Some(""), env_base: Some("env-base"), env_key: Some("env-key"), oidc: Some("oidc"), expected_base: "env-base", expected_key: Some("env-key") })]
#[case::empty_environment_falls_through(ProviderCase { api_base: None, api_key: None, env_base: Some(""), env_key: Some(""), oidc: Some("oidc"), expected_base: "https://ai-gateway.vercel.sh/v1", expected_key: Some("oidc") })]
#[case::empty_oidc_token_is_unset(ProviderCase { api_base: None, api_key: None, env_base: None, env_key: None, oidc: Some(""), expected_base: "https://ai-gateway.vercel.sh/v1", expected_key: None })]
fn provider_info_falls_through_empty_values_in_order(#[case] case: ProviderCase) {
    let resolved = provider_info(
        case.api_base,
        case.api_key.map(secret).as_ref(),
        &environment(case.env_base, case.env_key, case.oidc),
    );
    assert_eq!(resolved.api_base, case.expected_base);
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        case.expected_key
    );
}

#[rstest]
#[case::explicit_base(
    Some("https://explicit.example/v1/"),
    None,
    "https://explicit.example/v1/"
)]
#[case::empty_explicit_base(Some(""), Some("https://env.example/v1"), "https://env.example/v1")]
#[case::environment_base(None, Some("https://env.example/v1"), "https://env.example/v1")]
#[case::default_base(None, None, "https://ai-gateway.vercel.sh/v1")]
fn models_url_joins_the_resolved_base_without_trimming(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected_base: &str,
) {
    let resolved_environment = environment(env_base, None, None);
    let resolved = provider_info(api_base, None, &resolved_environment);
    assert_eq!(resolved.api_base, expected_base);
    assert_eq!(
        models_url(api_base, &resolved_environment),
        format!("{}/models", resolved.api_base)
    );
}

#[rstest]
#[case::non_catalog("custom-model", &[], &[], true, false)]
#[case::catalog_chat("gpt-4o", &["gpt-4o"], &[], true, true)]
#[case::catalog_text("text-davinci-003", &[], &["text-davinci-003"], true, true)]
#[case::old_gpt_has_no_response_format("gpt-4", &[], &[], false, false)]
fn supported_params_keep_the_base_order_then_extra_body(
    #[case] model: &str,
    #[case] chat: &[&str],
    #[case] text: &[&str],
    #[case] has_response_format: bool,
    #[case] has_user: bool,
) {
    let supported = config().supported_params(model, &catalog(chat, text));
    assert_eq!(supported.first(), Some(&"frequency_penalty"));
    assert_eq!(supported.get(1), Some(&"logit_bias"));
    assert_eq!(supported.last(), Some(&"extra_body"));
    assert_eq!(
        supported
            .iter()
            .filter(|name| **name == "extra_body")
            .count(),
        1
    );
    assert_eq!(supported.contains(&"response_format"), has_response_format);
    assert_eq!(supported.contains(&"user"), has_user);
    assert!(!supported.contains(&"reasoning_effort"));
}

#[test]
fn provider_options_replace_the_caller_extra_body() {
    let mut requested = object(
        r#"{"providerOptions":{"gateway":{"order":["a"]}},"extra_body":{"caller":true},"temperature":0.5,"unsupported":1}"#,
    );
    let mut optional = object(r#"{"existing":true}"#);
    config().map_openai_params(
        "custom-model",
        &mut requested,
        &mut optional,
        &catalog(&[], &[]),
    );
    assert_eq!(
        optional,
        object(
            r#"{"extra_body":{"providerOptions":{"gateway":{"order":["a"]}}},"temperature":0.5,"existing":true}"#
        )
    );
    assert_eq!(
        requested,
        object(r#"{"extra_body":{"caller":true},"temperature":0.5,"unsupported":1}"#)
    );
}

#[rstest]
#[case::null_provider_options(r#"{"providerOptions":null}"#)]
#[case::absent_provider_options(r#"{"extra_body":{"caller":true}}"#)]
#[case::absent_everything("{}")]
fn extra_body_is_always_replaced(#[case] requested_json: &str) {
    let mut requested = object(requested_json);
    let mut optional = object(r#"{"extra_body":{"stale":true}}"#);
    config().map_openai_params(
        "custom-model",
        &mut requested,
        &mut optional,
        &catalog(&[], &[]),
    );
    assert_eq!(
        optional.get("extra_body"),
        Some(&Value::Object(Object::new()))
    );
}

#[test]
fn the_base_mapping_drops_reasoning_effort_like_the_gpt_policy() {
    let mut requested = object(r#"{"reasoning_effort":"high","temperature":0.2}"#);
    let mut optional = Object::new();
    config().map_openai_params(
        "custom-model",
        &mut requested,
        &mut optional,
        &catalog(&[], &[]),
    );
    assert_eq!(optional, object(r#"{"temperature":0.2,"extra_body":{}}"#));
}

#[rstest]
#[case::authorization_for_a_key(Some("embed-key"), "{}", Some("Bearer embed-key"), None)]
#[case::no_authorization_without_a_key(None, "{}", None, None)]
#[case::empty_key_adds_nothing(Some(""), "{}", None, None)]
#[case::caller_key_overrides_the_default(
    Some("embed-key"),
    r#"{"Authorization":"Bearer caller"}"#,
    Some("Bearer caller"),
    None
)]
#[case::differently_cased_caller_key_is_a_second_entry(
    Some("embed-key"),
    r#"{"content-type":"text/plain"}"#,
    Some("Bearer embed-key"),
    Some("text/plain")
)]
fn embedding_headers_start_from_json_and_merge_the_caller(
    #[case] api_key: Option<&str>,
    #[case] caller_json: &str,
    #[case] expected_authorization: Option<&str>,
    #[case] expected_lower_content_type: Option<&str>,
) {
    let caller_headers: Headers = serde_json::from_str(caller_json).expect("caller headers");
    let headers = validate_environment(&caller_headers, api_key.map(secret).as_ref());
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        expected_authorization
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        expected_lower_content_type
    );
}

#[rstest]
#[case::explicit_base_trims(
    Some("https://explicit.example/v1///"),
    None,
    "https://explicit.example/v1/embeddings"
)]
#[case::empty_explicit_uses_default(Some(""), None, "https://ai-gateway.vercel.sh/v1/embeddings")]
#[case::environment_base_is_not_trimmed(
    None,
    Some("https://env.example/v1/"),
    "https://env.example/v1//embeddings"
)]
#[case::empty_environment_uses_default(
    None,
    Some(""),
    "https://ai-gateway.vercel.sh/v1/embeddings"
)]
#[case::default(None, None, "https://ai-gateway.vercel.sh/v1/embeddings")]
fn complete_url_trims_only_an_explicit_base(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        get_complete_url(api_base, &environment(env_base, None, None)),
        expected
    );
}

#[rstest]
#[case::one_prefix("vercel_ai_gateway/text-embedding-3-small", "text-embedding-3-small")]
#[case::two_prefixes("vercel_ai_gateway/vercel_ai_gateway/model", "vercel_ai_gateway/model")]
fn a_string_input_is_wrapped_and_the_prefix_is_stripped_once(
    #[case] model: &str,
    #[case] expected_model: &str,
) {
    let body = transform_embedding_request(
        model,
        &EmbeddingInput::Text(String::from("hello")),
        &Object::new(),
    );
    assert_eq!(body.get("model"), Some(&json!(expected_model)));
    assert_eq!(body.get("input"), Some(&json!(["hello"])));
}

#[test]
fn a_prefix_that_is_not_at_the_start_is_kept() {
    let body = transform_embedding_request(
        "openai/vercel_ai_gateway/model",
        &EmbeddingInput::Text(String::from("hello")),
        &Object::new(),
    );
    assert_eq!(
        body.get("model"),
        Some(&json!("openai/vercel_ai_gateway/model"))
    );
}

#[test]
fn options_overlay_model_and_input() {
    let optional = object(r#"{"model":"override","input":"option","dimensions":8}"#);
    let body = transform_embedding_request(
        "vercel_ai_gateway/model",
        &EmbeddingInput::Text(String::from("hello")),
        &optional,
    );
    assert_eq!(body.get("model"), Some(&json!("override")));
    assert_eq!(body.get("input"), Some(&json!("option")));
    assert_eq!(body.get("dimensions"), Some(&json!(8_u32)));
}

#[rstest]
#[case::texts(EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]), json!(["a", "b"]))]
#[case::tokens(EmbeddingInput::Tokens(vec![1_u32, 2_u32]), json!([1_u32, 2_u32]))]
#[case::token_lists(EmbeddingInput::TokenLists(vec![vec![1_u32, 2_u32], vec![3_u32]]), json!([[1_u32, 2_u32], [3_u32]]))]
#[case::empty_list(EmbeddingInput::Texts(Vec::new()), json!([]))]
fn non_string_inputs_keep_their_shape(#[case] input: EmbeddingInput, #[case] expected: Value) {
    let body = transform_embedding_request("model", &input, &Object::new());
    assert_eq!(body.get("input"), Some(&expected));
}

#[test]
fn embedding_params_copy_only_the_four_supported_keys() {
    let requested = object(
        r#"{"timeout":30,"dimensions":8,"encoding_format":"float","user":"reader","model":"m","input":"x","drop_params":true}"#,
    );
    let mut optional = object(r#"{"existing":true}"#);
    map_embedding_params(&requested, &mut optional);
    assert_eq!(
        optional,
        object(
            r#"{"existing":true,"timeout":30,"dimensions":8,"encoding_format":"float","user":"reader"}"#
        )
    );
}

#[test]
fn embedding_params_overwrite_existing_supported_values() {
    let mut optional = object(r#"{"dimensions":4}"#);
    map_embedding_params(&object(r#"{"dimensions":8}"#), &mut optional);
    assert_eq!(optional.get("dimensions"), Some(&json!(8_u32)));
}

#[test]
fn an_embedding_reply_is_handed_to_the_hook_before_decoding() {
    let reply = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,-1]}],"model":"upstream","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
    let seen = RefCell::new(Vec::new());
    let mut hook = |text: &str| seen.borrow_mut().push(text.to_owned());
    let decoded = transform_embedding_response(reply.as_bytes(), &mut hook).expect("reply");
    assert_eq!(decoded.model, "upstream");
    assert_eq!(decoded.data.len(), 1);
    assert_eq!(seen.into_inner(), vec![reply.to_owned()]);
}

#[rstest]
#[case::invalid_json(b"{", 1)]
#[case::wrong_shape(br#"{"object":"list"}"#, 1)]
#[case::not_an_object(b"[]", 1)]
#[case::invalid_utf8(b"\xff\xfe", 0)]
fn a_malformed_embedding_reply_is_a_typed_error(#[case] raw: &[u8], #[case] hook_calls: usize) {
    let seen = RefCell::new(Vec::new());
    let mut hook = |text: &str| seen.borrow_mut().push(text.to_owned());
    let error = transform_embedding_response(raw, &mut hook).expect_err("malformed");
    assert!(matches!(error, EmbeddingError::Decode { .. }));
    assert_eq!(seen.into_inner().len(), hook_calls);
}

#[test]
fn model_ids_keep_their_order_and_duplicates() {
    let models =
        parse_models(200, br#"{"data":[{"id":"b"},{"id":"a"},{"id":"b"}]}"#).expect("list");
    assert_eq!(models, vec!["b", "a", "b"]);
}

#[test]
fn an_empty_model_list_parses_to_no_ids() {
    assert!(
        parse_models(200, br#"{"data":[]}"#)
            .expect("list")
            .is_empty()
    );
}

#[rstest]
#[case::missing_data(b"{}")]
#[case::data_is_an_object(br#"{"data":{}}"#)]
#[case::entry_without_id(br#"{"data":[{"name":"x"}]}"#)]
#[case::non_string_id(br#"{"data":[{"id":3}]}"#)]
#[case::top_level_array(b"[]")]
#[case::invalid_json(b"{")]
fn a_model_list_of_the_wrong_shape_is_a_typed_error(#[case] body: &[u8]) {
    let error = parse_models(200, body).expect_err("malformed");
    assert!(matches!(error, ModelsError::Shape(_)));
    assert_eq!(error.status_code(), 502);
    assert!(error.source().is_some());
}

#[test]
fn only_status_200_counts_as_a_model_list() {
    let error = parse_models(201, br#"{"data":[]}"#).expect_err("status");
    assert!(matches!(error, ModelsError::Status { code: 201, .. }));
}

#[test]
fn a_non_200_model_list_keeps_litellm_failure_text_and_status() {
    let error = parse_models(500, b"upstream down").expect_err("status");
    assert_eq!(error.to_string(), "Failed to get models: upstream down");
    assert_eq!(error.status_code(), 500);
}

#[test]
fn a_non_200_model_list_with_invalid_utf8_names_the_bytes() {
    let error = parse_models(502, &[0xff, 0xfe]).expect_err("status");
    assert_eq!(
        error.to_string(),
        "Failed to get models: 2 bytes that are not UTF-8"
    );
    assert_eq!(error.status_code(), 502);
}

#[test]
fn the_error_class_carries_the_three_values_and_nothing_else() {
    let headers = Headers::from([(String::from("x-request-id"), String::from("req_1"))]);
    let error = get_error_class("upstream failed", 429, &headers);
    assert_eq!(error.message, "upstream failed");
    assert_eq!(error.status_code, 429);
    assert_eq!(error.headers, headers);
    assert_eq!(error.to_string(), "upstream failed");
}

#[test]
fn the_config_reports_the_litellm_provider_name() {
    assert_eq!(VercelAiGatewayConfig::PROVIDER, "vercel_ai_gateway");
}

#[rstest::rstest]
fn replays_the_vercel_ai_gateway_reply_corpus(
    #[files("fuzz/corpus/vercel_ai_gateway_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _reply = decode_embedding_response(&bytes);
    let _ok = parse_models(200, &bytes);
    let _failed = parse_models(503, &bytes);
}
