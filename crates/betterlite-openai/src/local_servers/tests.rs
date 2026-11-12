#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::Object as PromptObject;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::error::LocalServerError;
use super::oobabooga::error::{OobaboogaError, ReplyError};
use super::oobabooga::reply::{ChatReply, EmbeddingReply};
use super::{ProviderEnvironment, docker_model_runner, lemonade_cost, llamafile, lm_studio};
use super::{oobabooga, xinference_image};
use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::embedding::EmbeddingInput;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).expect("json")
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn object(value: Value) -> Object {
    serde_json::from_value(value).expect("an object")
}

fn base_config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn no_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn messages(value: Value) -> Vec<ChatMessage> {
    serde_json::from_value(value).expect("chat messages")
}

type Resolved = Vec<(Option<String>, String)>;

fn environment(base: &str, key: &str) -> ProviderEnvironment {
    ProviderEnvironment {
        api_base: Some(base.to_owned()),
        api_key: Some(secret(key)),
    }
}

fn resolve_all<Resolve>(resolve: Resolve) -> Resolved
where
    Resolve:
        Fn(Option<&str>, Option<&SecretString>, &ProviderEnvironment) -> (Option<String>, String),
{
    let blank = secret("");
    vec![
        resolve(None, None, &ProviderEnvironment::default()),
        resolve(
            Some("http://x"),
            Some(&secret("k")),
            &environment("http://env", "env-key"),
        ),
        resolve(None, None, &environment("http://env", "env-key")),
        resolve(
            Some(""),
            Some(&blank),
            &environment("http://env", "env-key"),
        ),
        resolve(Some(""), Some(&blank), &environment("", "")),
    ]
}

fn pairs(expected: &[(Option<&str>, &str)]) -> Resolved {
    expected
        .iter()
        .map(|(base, key)| (base.map(str::to_owned), (*key).to_owned()))
        .collect()
}

#[test]
fn docker_provider_info_resolves_base_and_key() {
    let got = resolve_all(|base, key, environment| {
        let info = docker_model_runner::provider_info(base, key, environment);
        (Some(info.api_base), info.api_key.expose_secret().to_owned())
    });
    let default = Some("http://localhost:22088/engines/llama.cpp");
    let expected = pairs(&[
        (default, "dummy-key"),
        (Some("http://x"), "k"),
        (Some("http://env"), "env-key"),
        (Some("http://env"), "env-key"),
        (default, "dummy-key"),
    ]);
    assert_eq!(got, expected);
}

#[test]
fn llamafile_provider_info_resolves_base_and_key() {
    let got = resolve_all(|base, key, environment| {
        let info = llamafile::provider_info(base, key, environment);
        (Some(info.api_base), info.api_key.expose_secret().to_owned())
    });
    let default = Some("http://127.0.0.1:8080/v1");
    let expected = pairs(&[
        (default, "fake-api-key"),
        (Some("http://x"), "k"),
        (Some("http://env"), "env-key"),
        (Some("http://env"), "env-key"),
        (default, "fake-api-key"),
    ]);
    assert_eq!(got, expected);
}

#[test]
fn lm_studio_provider_info_has_no_default_base() {
    let got = resolve_all(|base, key, environment| {
        let info = lm_studio::provider_info(base, key, environment);
        (info.api_base, info.api_key.expose_secret().to_owned())
    });
    let expected = pairs(&[
        (None, "fake-api-key"),
        (Some("http://x"), "k"),
        (Some("http://env"), "env-key"),
        (Some("http://env"), "env-key"),
        (None, "fake-api-key"),
    ]);
    assert_eq!(got, expected);
}

#[rstest]
#[case(None, "http://localhost:22088/engines/llama.cpp/v1/chat/completions")]
#[case(
    Some(""),
    "http://localhost:22088/engines/llama.cpp/v1/chat/completions"
)]
#[case(
    Some("http://h/engines/vllm"),
    "http://h/engines/vllm/v1/chat/completions"
)]
#[case(
    Some("http://h/engines/vllm///"),
    "http://h/engines/vllm/v1/chat/completions"
)]
fn docker_complete_url(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(docker_model_runner::complete_url(api_base), expected);
}

fn docker_messages(value: Value) -> Value {
    let out = docker_model_runner::transform_messages("m", &messages(value), &RefuseDownloads)
        .expect("transformed messages");
    serde_json::to_value(out).expect("json")
}

#[test]
fn docker_transform_joins_text_parts_into_a_string() {
    let got = docker_messages(parse_json(
        r#"[
        {"role": "user", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]},
        {"role": "user", "content": "plain"}
    ]"#,
    ));
    let expected = parse_json(
        r#"[
        {"role": "user", "content": "ab"},
        {"role": "user", "content": "plain"}
    ]"#,
    );
    assert_eq!(got, expected);
}

#[test]
fn docker_transform_keeps_image_only_lists_and_cache_control() {
    let input = parse_json(
        r#"[
        {"role": "user", "content": [{"type": "image_url", "image_url": {"url": "http://i/x.png"}}]},
        {"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}
    ]"#,
    );
    assert_eq!(docker_messages(input.clone()), input);
}

#[test]
fn docker_transform_reports_bad_file_part() {
    let bad = messages(parse_json(
        r#"[
        {"role": "user", "content": [{"type": "file"}]}
    ]"#,
    ));
    let result = docker_model_runner::transform_messages("m", &bad, &RefuseDownloads);
    assert!(matches!(result, Err(LocalServerError::Content(_))));
}

fn docker_map(model: &str, params: &[(&str, Value)]) -> Object {
    let requested: IndexMap<String, Value> = params
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    let config = docker_model_runner::ChatConfig {
        base: base_config(),
    };
    let mut optional = Object::new();
    config.map_openai_params(model, &requested, &mut optional, &no_catalog());
    optional
}

#[test]
fn docker_map_renames_max_completion_tokens_in_order() {
    let later = docker_map(
        "m",
        &[
            ("max_tokens", parse_json("1")),
            ("max_completion_tokens", parse_json("2")),
        ],
    );
    assert_eq!(Value::Object(later), parse_json(r#"{"max_tokens": 2}"#));
    let earlier = docker_map(
        "m",
        &[
            ("max_completion_tokens", parse_json("2")),
            ("max_tokens", parse_json("1")),
        ],
    );
    assert_eq!(Value::Object(earlier), parse_json(r#"{"max_tokens": 1}"#));
}

#[test]
fn docker_map_copies_only_supported_params() {
    let got = docker_map(
        "m",
        &[
            ("temperature", parse_json("0.5")),
            ("reasoning_effort", parse_json(r#""low""#)),
            ("top_k", parse_json("3")),
            ("response_format", parse_json(r#"{"type": "json_object"}"#)),
        ],
    );
    let expected =
        parse_json(r#"{"temperature": 0.5, "response_format": {"type": "json_object"}}"#);
    assert_eq!(Value::Object(got), expected);
    let gpt4 = docker_map(
        "gpt-4",
        &[("response_format", parse_json(r#"{"type": "text"}"#))],
    );
    assert!(gpt4.is_empty());
}

fn lm_studio_map(model: &str, params: Value) -> Value {
    let config = lm_studio::ChatConfig {
        base: base_config(),
    };
    let mut optional = Object::new();
    config.map_openai_params(model, &object(params), &mut optional, &no_catalog());
    Value::Object(optional)
}

fn params_with(format: Value) -> Value {
    let mut params = Object::new();
    params.insert(String::from("response_format"), format);
    params.insert(String::from("temperature"), parse_json("1"));
    Value::Object(params)
}

#[rstest]
#[case(
    parse_json(r#"{"type": "json_schema", "schema": {"a": 1}}"#),
    parse_json(r#"{"type": "json_schema", "json_schema": {"schema": {"a": 1}}}"#)
)]
#[case(
    parse_json(r#"{"type": "json_schema", "schema": {"a": 1}, "json_schema": {"name": "n"}}"#),
    parse_json(r#"{"type": "json_schema", "schema": {"a": 1}, "json_schema": {"name": "n"}}"#)
)]
#[case(
    parse_json(r#"{"type": "json_schema"}"#),
    parse_json(r#"{"type": "json_schema"}"#)
)]
#[case(
    parse_json(r#"{"type": "json_object", "x": 1}"#),
    parse_json(r#"{"type": "json_object", "x": 1}"#)
)]
#[case(parse_json(r#"{"type": "text"}"#), parse_json(r#"{"type": "text"}"#))]
#[case(parse_json(r#""raw""#), parse_json(r#""raw""#))]
fn lm_studio_response_format(#[case] format: Value, #[case] expected: Value) {
    let got = lm_studio_map("m", params_with(format));
    assert_eq!(got, params_with(expected));
}

#[test]
fn lm_studio_json_formats_bypass_the_gpt4_exclusion() {
    let schema = parse_json(r#"{"type": "json_schema", "schema": {}}"#);
    let rewritten = parse_json(r#"{"type": "json_schema", "json_schema": {"schema": {}}}"#);
    assert_eq!(
        lm_studio_map("gpt-4", params_with(schema)),
        params_with(rewritten)
    );
    let json_object = parse_json(r#"{"type": "json_object"}"#);
    assert_eq!(
        lm_studio_map("gpt-4", params_with(json_object.clone())),
        params_with(json_object)
    );
    let text = parse_json(r#"{"type": "text"}"#);
    assert_eq!(
        lm_studio_map("gpt-4", params_with(text)),
        parse_json(r#"{"temperature": 1}"#)
    );
}

#[test]
fn xinference_map_copies_supported_and_keeps_existing() {
    let mut optional = object(parse_json(r#"{"size": "kept"}"#));
    let requested = object(parse_json(
        r#"{"n": 2, "size": "new", "response_format": "b64_json"}"#,
    ));
    xinference_image::map_openai_params("m", &requested, &mut optional, false).expect("mapped");
    let expected = parse_json(r#"{"n": 2, "size": "kept", "response_format": "b64_json"}"#);
    assert_eq!(Value::Object(optional), expected);
}

#[test]
fn xinference_unsupported_param_drops_or_fails() {
    let requested = object(parse_json(r#"{"quality": "hd", "n": 1}"#));
    let mut dropped = Object::new();
    xinference_image::map_openai_params("m", &requested, &mut dropped, true).expect("dropped");
    assert_eq!(Value::Object(dropped), parse_json(r#"{"n": 1}"#));
    let mut failed = Object::new();
    let error = xinference_image::map_openai_params("sd", &requested, &mut failed, false)
        .expect_err("unsupported");
    assert!(matches!(
        &error,
        LocalServerError::UnsupportedParam { param, model, supported }
            if param == "quality" && model == "sd" && supported.len() == 4
    ));
    assert!(
        error
            .to_string()
            .starts_with("Parameter quality is not supported for model sd.")
    );
    let mut existing = object(parse_json(r#"{"quality": "kept"}"#));
    xinference_image::map_openai_params("sd", &requested, &mut existing, false).expect("kept");
    assert_eq!(
        Value::Object(existing),
        parse_json(r#"{"quality": "kept", "n": 1}"#)
    );
}

#[test]
fn xinference_supported_params_repeat_response_format() {
    assert_eq!(
        xinference_image::SUPPORTED_PARAMS,
        ["n", "response_format", "size", "response_format"]
    );
}

#[test]
fn lemonade_cost_is_zero_for_both_sides() {
    assert_eq!(
        lemonade_cost::COST_PER_TOKEN,
        (Decimal::ZERO, Decimal::ZERO)
    );
}

#[test]
fn oobabooga_headers_carry_token_only_with_a_key() {
    let bare = oobabooga::validate_environment(None);
    let expected = Headers::from([
        ("accept".to_owned(), "application/json".to_owned()),
        ("content-type".to_owned(), "application/json".to_owned()),
    ]);
    assert_eq!(bare, expected);
    let keyed = oobabooga::validate_environment(Some(&secret("k")));
    assert_eq!(
        keyed.get("Authorization").map(String::as_str),
        Some("Token k")
    );
    assert_eq!(keyed.len(), 3);
    let empty = oobabooga::validate_environment(Some(&secret("")));
    assert_eq!(
        empty.get("Authorization").map(String::as_str),
        Some("Token ")
    );
}

type UrlFn = fn(&str, Option<&str>) -> Result<String, OobaboogaError>;

#[rstest]
#[case(
    oobabooga::completion_url,
    "m",
    Some("http://h:5000"),
    Ok("http://h:5000/v1/chat/completions")
)]
#[case(
    oobabooga::completion_url,
    "http://m:1",
    Some("http://h"),
    Ok("http://m:1/v1/chat/completions")
)]
#[case(
    oobabooga::completion_url,
    "https://m",
    None,
    Ok("https://m/v1/chat/completions")
)]
#[case(
    oobabooga::completion_url,
    "httpx",
    Some("http://h"),
    Ok("http://h/v1/chat/completions")
)]
#[case(oobabooga::completion_url, "m", None, Err(()))]
#[case(oobabooga::completion_url, "m", Some(""), Err(()))]
#[case(
    oobabooga::embeddings_url,
    "m",
    Some("http://h"),
    Ok("http://h/v1/embeddings")
)]
#[case(
    oobabooga::embeddings_url,
    "http://m/e",
    Some("http://h"),
    Ok("http://m/e")
)]
#[case(oobabooga::embeddings_url, "https://m/e", None, Ok("https://m/e"))]
#[case(oobabooga::embeddings_url, "m", None, Err(()))]
#[case(oobabooga::embeddings_url, "m", Some(""), Err(()))]
fn oobabooga_urls(
    #[case] resolve: UrlFn,
    #[case] model: &str,
    #[case] api_base: Option<&str>,
    #[case] expected: Result<&str, ()>,
) {
    let got = resolve(model, api_base).map_err(|error| {
        assert_eq!(error.status_code, 404);
        assert_eq!(
            error.message,
            "API Base not set. Set one via completion(..,api_base='your-api-url')"
        );
    });
    assert_eq!(got, expected.map(str::to_owned));
}

fn prompt_object(value: Value) -> PromptObject {
    serde_json::from_value(value).expect("an object")
}

#[test]
fn oobabooga_completion_body_strips_cache_control_and_retries() {
    let chat = messages(parse_json(
        r#"[
        {"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}
    ]"#,
    ));
    let params = prompt_object(parse_json(r#"{"temperature": 0.5, "max_retries": 3}"#));
    let body = oobabooga::completion_body("m", &chat, &params, &RefuseDownloads).expect("a body");
    let expected = parse_json(
        r#"{
        "model": "m",
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.5
    }"#,
    );
    assert_eq!(serde_json::to_value(body).expect("json"), expected);
}

#[rstest]
#[case(EmbeddingInput::Text("a".to_owned()), parse_json("{}"), parse_json(r#"{"input": "a"}"#))]
#[case(
    EmbeddingInput::Texts(vec!["a".to_owned(), "b".to_owned()]),
    parse_json(r#"{"dimensions": 3}"#),
    parse_json(r#"{"input": ["a", "b"], "dimensions": 3}"#)
)]
#[case(EmbeddingInput::Tokens(vec![1, 2]), parse_json("{}"), parse_json(r#"{"input": [1, 2]}"#))]
#[case(EmbeddingInput::Text("a".to_owned()), parse_json(r#"{"input": "over"}"#), parse_json(r#"{"input": "over"}"#))]
fn oobabooga_embedding_body(
    #[case] input: EmbeddingInput,
    #[case] optional: Value,
    #[case] expected: Value,
) {
    let body = oobabooga::embedding_body(&input, &prompt_object(optional)).expect("a body");
    assert_eq!(serde_json::to_value(body).expect("json"), expected);
}

fn chat_reply(raw: &str, status: u16) -> Result<ChatReply, ReplyError> {
    oobabooga::reply::transform_response("m", status, raw, 7)
}

fn server_error<T>(result: Result<T, ReplyError>) -> OobaboogaError {
    match result {
        Err(ReplyError::Server(error)) => error,
        _ => panic!("expected an OobaboogaError"),
    }
}

const GOOD_USAGE: &str = r#""usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#;

#[test]
fn oobabooga_reply_reads_content_and_usage() {
    let raw = format!(r#"{{"choices":[{{"message":{{"content":"hello"}}}}],{GOOD_USAGE}}}"#);
    let reply = chat_reply(&raw, 200).expect("a reply");
    let expected = parse_json(
        r#"{
        "model": "m",
        "created": 7,
        "content": "hello",
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
    }"#,
    );
    assert_eq!(serde_json::to_value(reply).expect("json"), expected);
    let null_content = format!(r#"{{"choices":[{{"message":{{"content":null}}}}],{GOOD_USAGE}}}"#);
    assert_eq!(
        chat_reply(&null_content, 200).expect("a reply").content,
        None
    );
}

#[rstest]
#[case(r#"{"error":"boom","choices":[]}"#, 502, "boom")]
#[case(r#"{"error":{"message":"x"}}"#, 400, r#"{"message":"x"}"#)]
#[case(r#"{"error":null}"#, 500, "null")]
#[case("not json", 503, "not json")]
#[case(r#"{"usage":{}}"#, 200, "'choices'")]
#[case(r#"{"choices":[]}"#, 200, "list index out of range")]
#[case(r#"{"choices":[{}]}"#, 200, "'message'")]
#[case(r#"{"choices":[{"message":{}}]}"#, 200, "'content'")]
fn oobabooga_reply_errors_keep_status_and_text(
    #[case] raw: &str,
    #[case] status: u16,
    #[case] message: &str,
) {
    let error = server_error(chat_reply(raw, status));
    assert_eq!(error.status_code, status);
    assert_eq!(error.message, message);
    assert_eq!(error.headers, None);
    assert_eq!(error.to_string(), message);
}

#[test]
fn oobabooga_reply_without_usage_or_shape_is_a_typed_error() {
    let no_usage = r#"{"choices":[{"message":{"content":"x"}}]}"#;
    assert!(matches!(
        chat_reply(no_usage, 200),
        Err(ReplyError::Missing { field: "usage" })
    ));
    let partial = r#"{"choices":[{"message":{"content":"x"}}],"usage":{"prompt_tokens":1}}"#;
    assert!(matches!(
        chat_reply(partial, 200),
        Err(ReplyError::Shape { .. })
    ));
    assert!(matches!(
        chat_reply("[1]", 200),
        Err(ReplyError::Shape { .. })
    ));
    let wrong_type = r#"{"choices":"x"}"#;
    assert!(matches!(
        chat_reply(wrong_type, 200),
        Err(ReplyError::Shape { .. })
    ));
}

fn embedding_reply(raw: &str) -> Result<EmbeddingReply, ReplyError> {
    oobabooga::reply::transform_embedding_response("m", raw)
}

#[test]
fn oobabooga_embedding_reply_counts_vector_length() {
    let reply = embedding_reply(r#"{"data":[{"embedding":[0.5,1,2]},{"embedding":[9]}]}"#)
        .expect("a reply");
    let expected = parse_json(
        r#"{"model": "m", "embedding": [0.5, 1, 2], "prompt_tokens": 3, "total_tokens": 3}"#,
    );
    assert_eq!(serde_json::to_value(reply).expect("json"), expected);
}

#[rstest]
#[case(r#"{"error":"bad","status_code":429}"#, 429, "bad")]
#[case(r#"{"error":"bad"}"#, 500, "bad")]
#[case(r#"{"error":"bad","status_code":null}"#, 500, "bad")]
#[case(r#"{"error":{"a":1}}"#, 500, r#"{"a":1}"#)]
fn oobabooga_embedding_reply_error(#[case] raw: &str, #[case] status: u16, #[case] message: &str) {
    let error = server_error(embedding_reply(raw));
    assert_eq!(error.status_code, status);
    assert_eq!(error.message, message);
}

#[test]
fn oobabooga_embedding_reply_rejects_missing_parts() {
    assert!(matches!(
        embedding_reply("{}"),
        Err(ReplyError::Missing { field: "data[0]" })
    ));
    assert!(matches!(
        embedding_reply(r#"{"data":[]}"#),
        Err(ReplyError::Missing { field: "data[0]" })
    ));
    assert!(matches!(
        embedding_reply(r#"{"data":[{}]}"#),
        Err(ReplyError::Missing { field: "embedding" })
    ));
    assert!(matches!(
        embedding_reply("nope"),
        Err(ReplyError::Shape { .. })
    ));
}
