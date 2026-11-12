#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::cometapi_embedding_credentials::Settings;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, Value};

use super::chat::{CHAT_FAMILY, ChatError, ChatTransform, complete_url, transform_request};
use super::chunk::chunk_parser;
use super::embed::{
    CometEmbeddingReply, MAX_REPLY_BYTES, complete_url as embedding_url,
    decode_comet_embedding_reply, map_openai_params as map_embedding_params,
    transform_embedding_request, validate_environment,
};
use super::error::{CometApiError, get_error_class};
use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

fn expected(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn chat_messages(source: &str) -> Vec<ChatMessage> {
    serde_json::from_str(source).unwrap()
}

fn headers_of(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn json_headers() -> Headers {
    headers_of(&[("Content-Type", "application/json")])
}

fn body_value(body: &Object) -> Value {
    serde_json::to_value(body).unwrap()
}

fn chunk_value(chunk: &Json) -> Value {
    serde_json::to_value(chunk).unwrap()
}

#[rstest]
#[case::absent_base(None, "https://api.cometapi.com/v1/chat/completions")]
#[case::empty_base(Some(""), "/v1/chat/completions")]
#[case::official_host(
    Some("https://api.cometapi.com"),
    "https://api.cometapi.com/v1/chat/completions"
)]
#[case::official_host_with_slash(
    Some("https://api.cometapi.com/"),
    "https://api.cometapi.com/v1/chat/completions"
)]
#[case::ending_in_v1(
    Some("https://proxy.example/v1"),
    "https://proxy.example/v1/chat/completions"
)]
#[case::ending_in_v1_slash(
    Some("https://proxy.example/v1/"),
    "https://proxy.example/v1/chat/completions"
)]
#[case::recognized_v10(
    Some("https://proxy.example/v10"),
    "https://proxy.example/v10/chat/completions"
)]
#[case::complete_endpoint(
    Some("https://proxy.example/v1/chat/completions"),
    "https://proxy.example/v1/chat/completions"
)]
#[case::endpoint_with_suffix(
    Some("https://proxy.example/chat/completions/extra"),
    "https://proxy.example/chat/completions/extra"
)]
#[case::endpoint_in_query(
    Some("https://proxy.example/x?q=chat/completions"),
    "https://proxy.example/x?q=chat/completions"
)]
#[case::version_in_query(
    Some("https://proxy.example/x?q=/v1"),
    "https://proxy.example/x?q=/v1/chat/completions"
)]
#[case::version_after_a_double_slash(Some("a//b/v1"), "a//b/v1/chat/completions")]
#[case::double_slash_without_version(Some("a//b"), "a//b/v1/chat/completions")]
#[case::scheme_relative_version(Some("//host/v1"), "//host/v1/chat/completions")]
#[case::scheme_relative_host(Some("//host"), "//host/v1/chat/completions")]
#[case::spaces_are_kept(Some("  "), "  /v1/chat/completions")]
fn chat_complete_url_matches_the_pinned_rules(
    #[case] api_base: Option<&str>,
    #[case] expected_url: &str,
) {
    assert_eq!(complete_url(api_base), expected_url);
}

#[test]
fn transform_request_overlays_extra_body_last() {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional = object(
        r#"{"temperature":0.5,"extra_body":{"a":1,"model":"over","messages":[],"temperature":0.9}}"#,
    );
    let environment = BTreeMap::new();
    let transform = ChatTransform {
        model: "m",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let body = transform_request(&transform).unwrap();
    assert_eq!(
        body_value(&body),
        expected(r#"{"model": "over", "messages": [], "temperature": 0.9, "a": 1}"#)
    );
}

#[rstest]
#[case::absent(r#"{"temperature":0.5}"#)]
#[case::empty(r#"{"temperature":0.5,"extra_body":{}}"#)]
fn transform_request_without_a_filled_extra_body_keeps_the_parent_body(#[case] source: &str) {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional = object(source);
    let environment = BTreeMap::new();
    let transform = ChatTransform {
        model: "m",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let body = transform_request(&transform).unwrap();
    assert_eq!(
        body_value(&body),
        expected(
            r#"{"model": "m", "messages": [{"role": "user", "content": "hi"}], "temperature": 0.5}"#
        )
    );
}

#[rstest]
#[case::null("null")]
#[case::number("5")]
#[case::text("\"ab\"")]
#[case::boolean("true")]
#[case::list("[1]")]
#[case::pairs("[[\"a\",1]]")]
fn transform_request_rejects_a_present_non_object_extra_body(#[case] extra_body: &str) {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional = object(&format!("{{\"extra_body\":{extra_body}}}"));
    let environment = BTreeMap::new();
    let transform = ChatTransform {
        model: "m",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    assert!(matches!(
        transform_request(&transform),
        Err(ChatError::ExtraBody)
    ));
}

#[test]
fn transform_request_removes_cache_control_and_applies_the_parent_rules() {
    let messages = chat_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]},{"role":"user","content":"plain","cache_control":{"type":"ephemeral"}}]"#,
    );
    let optional = object(
        r#"{"temperature":0.5,"max_retries":2,"tool_choice":"auto","tools":[{"type":"function","function":{"name":"f","cache_control":{"x":1}}}]}"#,
    );
    let environment = BTreeMap::new();
    let transform = ChatTransform {
        model: "m",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let body = transform_request(&transform).unwrap();
    assert_eq!(
        body_value(&body),
        expected(
            r#"{
                "model": "m",
                "messages": [
                    {"role": "user", "content": [{"type": "text", "text": "hi"}]},
                    {"role": "user", "content": "plain"}
                ],
                "temperature": 0.5,
                "tool_choice": "auto",
                "tools": [{"type": "function", "function": {"name": "f"}}]
            }"#
        )
    );
}

#[test]
fn transform_request_drops_tool_choice_when_no_tools_are_supplied() {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional = object(r#"{"max_retries":2,"tool_choice":"auto"}"#);
    let environment = BTreeMap::new();
    let transform = ChatTransform {
        model: "m",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let body = transform_request(&transform).unwrap();
    assert_eq!(
        body_value(&body),
        expected(r#"{"model": "m", "messages": [{"role": "user", "content": "hi"}]}"#)
    );
}

#[test]
fn transform_request_does_not_change_the_callers_options() {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional = object(r#"{"extra_body":{"a":1},"max_retries":2}"#);
    let before = optional.clone();
    let environment = BTreeMap::new();
    let transform = ChatTransform {
        model: "m",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    transform_request(&transform).unwrap();
    assert_eq!(optional, before);
}

fn chat_config() -> OpenAiConfig {
    OpenAiConfig {
        family: CHAT_FAMILY,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::from(["gpt-4o".to_owned(), "gpt-4".to_owned()]),
        text_models: BTreeSet::from(["gpt-3.5-turbo-instruct".to_owned()]),
    }
}

#[test]
fn chat_option_policy_uses_the_gpt_base_list() {
    let config = chat_config();
    let capabilities = catalog();
    let gpt_4o = config.supported_params("gpt-4o", &capabilities);
    assert!(gpt_4o.contains(&"response_format"));
    assert_eq!(gpt_4o.last().copied(), Some("user"));
    let gpt_4 = config.supported_params("gpt-4", &capabilities);
    assert!(!gpt_4.contains(&"response_format"));
    assert_eq!(gpt_4.last().copied(), Some("user"));
    let instruct = config.supported_params("gpt-3.5-turbo-instruct", &capabilities);
    assert!(instruct.contains(&"response_format"));
    assert_eq!(instruct.last().copied(), Some("user"));
    let unknown = config.supported_params("comet-model", &capabilities);
    assert!(!unknown.contains(&"user"));
    assert_eq!(unknown.last().copied(), Some("response_format"));
}

#[test]
fn chat_option_mapping_filters_to_the_base_list_and_adds_no_extra_body() {
    let config = chat_config();
    let capabilities = catalog();
    let mut mapped = Map::from_iter([("existing".to_owned(), Value::Bool(true))]);
    let requested = Map::from_iter([
        ("temperature".to_owned(), expected("0.5")),
        (
            "response_format".to_owned(),
            expected(r#"{"type": "json_object"}"#),
        ),
        ("unknown".to_owned(), expected("1")),
        ("user".to_owned(), expected("\"u\"")),
    ]);
    config.map_params("comet-model", &requested, &mut mapped, &capabilities);
    assert_eq!(
        Value::Object(mapped),
        expected(
            r#"{"existing": true, "temperature": 0.5, "response_format": {"type": "json_object"}}"#
        )
    );
    assert_eq!(config.family, ConfigFamily::Other);
}

#[test]
fn chat_error_class_keeps_the_supplied_status_and_headers() {
    let supplied = headers_of(&[("X-Request-Id", "r")]);
    let error = get_error_class("boom", 429, &supplied);
    assert_eq!(error.message, "boom");
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.headers, supplied);
}

fn shape_field(failure: CometApiError) -> String {
    failure.message
}

fn chunk(source: &str) -> Object {
    object(source)
}

#[test]
fn chunk_parser_copies_reasoning_into_every_choice() {
    let parsed = chunk_parser(&chunk(
        r#"{"id":"a","created":1,"model":"m","choices":[
            {"index":0,"delta":{"reasoning":"first","content":"x"}},
            {"index":1,"delta":{"reasoning":null,"reasoning_content":"old"}},
            {"index":2,"delta":{"content":"plain"}},
            {"index":3,"delta":{"reasoning":"kept","reasoning_content":"old"}}
        ]}"#,
    ))
    .unwrap();
    assert_eq!(parsed.choices.len(), 4);
    let first = chunk_value(&parsed.choices[0]);
    assert_eq!(first["delta"]["reasoning"], expected("\"first\""));
    assert_eq!(first["delta"]["reasoning_content"], expected("\"first\""));
    let nulled = chunk_value(&parsed.choices[1]);
    assert_eq!(nulled["delta"]["reasoning_content"], Value::Null);
    assert_eq!(nulled["delta"]["reasoning"], Value::Null);
    let plain = chunk_value(&parsed.choices[2]);
    assert!(plain["delta"].get("reasoning_content").is_none());
    let kept = chunk_value(&parsed.choices[3]);
    assert_eq!(kept["delta"]["reasoning_content"], expected("\"kept\""));
    assert_eq!(kept["delta"]["reasoning"], expected("\"kept\""));
}

#[test]
fn chunk_parser_forces_the_chunk_object_and_carries_usage() {
    let parsed = chunk_parser(&chunk(
        r#"{"id":"a","created":7,"model":null,"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
    ))
    .unwrap();
    assert_eq!(parsed.id, "a");
    assert_eq!(parsed.object, "chat.completion.chunk");
    assert_eq!(parsed.created, 7);
    assert_eq!(parsed.model, None);
    assert_eq!(
        parsed.usage,
        Some(Json::Object(object(
            r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#
        )))
    );
}

#[rstest]
#[case::choices(r#"{"id":"a","created":1,"model":"m"}"#, "choices")]
#[case::id(r#"{"created":1,"model":"m","choices":[]}"#, "id")]
#[case::created(r#"{"id":"a","model":"m","choices":[]}"#, "created")]
#[case::model(r#"{"id":"a","created":1,"choices":[]}"#, "model")]
fn chunk_parser_requires_choices_id_created_and_model(#[case] source: &str, #[case] field: &str) {
    let error = chunk_parser(&chunk(source)).unwrap_err();
    assert_eq!(error.status_code, Some(400));
    assert_eq!(error.headers, json_headers());
    assert!(error.message.contains(field));
}

#[test]
fn chunk_parser_reports_a_chunk_error_before_the_choices() {
    let error = chunk_parser(&chunk(r#"{"error":{"message":"boom","code":429}}"#)).unwrap_err();
    assert_eq!(error.message, "CometAPI Error: boom");
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.headers, json_headers());
}

#[rstest]
#[case::missing(r#"{"error":{}}"#, Some(400))]
#[case::given(r#"{"error":{"code":429}}"#, Some(429))]
#[case::zero(r#"{"error":{"code":0}}"#, Some(0))]
#[case::null(r#"{"error":{"code":null}}"#, None)]
fn chunk_parser_defaults_only_a_missing_error_code(
    #[case] source: &str,
    #[case] expected_status: Option<u16>,
) {
    let error = chunk_parser(&chunk(source)).unwrap_err();
    assert_eq!(error.status_code, expected_status);
    assert_eq!(error.headers, json_headers());
}

#[test]
fn chunk_parser_defaults_only_a_missing_error_message() {
    let absent = chunk_parser(&chunk(r#"{"error":{}}"#)).unwrap_err();
    assert_eq!(absent.message, "CometAPI Error: Unknown error");
    let explicit_null = chunk_parser(&chunk(r#"{"error":{"message":null}}"#)).unwrap_err();
    assert_eq!(explicit_null.message, "CometAPI Error: null");
    let wrong_type = shape_field(chunk_parser(&chunk(r#"{"error":{"message":5}}"#)).unwrap_err());
    assert!(wrong_type.contains("error.message"));
}

#[rstest]
#[case::id(r#"{"id":1,"created":1,"model":"m","choices":[]}"#, "id")]
#[case::created(r#"{"id":"a","created":"x","model":"m","choices":[]}"#, "created")]
#[case::model(r#"{"id":"a","created":1,"model":5,"choices":[]}"#, "model")]
#[case::choices(r#"{"id":"a","created":1,"model":"m","choices":{}}"#, "choices")]
#[case::choice_item(r#"{"id":"a","created":1,"model":"m","choices":["x"]}"#, "choices")]
#[case::delta(
    r#"{"id":"a","created":1,"model":"m","choices":[{"index":0,"delta":5}]}"#,
    "delta"
)]
#[case::error(r#"{"error":"plain"}"#, "error")]
fn chunk_parser_names_a_wrong_typed_field(#[case] source: &str, #[case] field: &str) {
    assert_eq!(
        shape_field(chunk_parser(&chunk(source)).unwrap_err()),
        field
    );
}

#[rstest]
#[case::absent_base(None, "https://api.cometapi.com/v1/embeddings")]
#[case::empty_base(Some(""), "/embeddings")]
#[case::only_slashes(Some("/"), "/embeddings")]
#[case::versioned_base(
    Some("https://api.cometapi.com/v1"),
    "https://api.cometapi.com/v1/embeddings"
)]
#[case::versioned_base_with_slash(
    Some("https://api.cometapi.com/v1/"),
    "https://api.cometapi.com/v1/embeddings"
)]
#[case::spaces_are_kept(Some("  "), "  /embeddings")]
fn embedding_complete_url_appends_embeddings(
    #[case] api_base: Option<&str>,
    #[case] expected_url: &str,
) {
    assert_eq!(embedding_url(api_base), expected_url);
}

fn environment(key: Option<&str>) -> Settings {
    Settings {
        api_key: key.map(SecretString::from),
    }
}

#[rstest]
#[case::nothing(None, None, "Bearer None")]
#[case::environment_key(None, Some("env-key"), "Bearer env-key")]
#[case::explicit_key(Some("explicit"), Some("env-key"), "Bearer explicit")]
#[case::explicit_empty(Some(""), Some("env-key"), "Bearer ")]
fn embedding_headers_read_only_the_primary_key(
    #[case] api_key: Option<&str>,
    #[case] primary_key: Option<&str>,
    #[case] authorization: &str,
) {
    let key = api_key.map(SecretString::from);
    let headers = validate_environment(&Headers::new(), key.as_ref(), &environment(primary_key));
    assert_eq!(
        headers,
        headers_of(&[
            ("Authorization", authorization),
            ("accept", "application/json"),
            ("Content-Type", "application/json")
        ])
    );
}

#[test]
fn embedding_headers_let_the_caller_override_each_default() {
    let settings = environment(Some("env-key"));
    let caller = headers_of(&[
        ("Authorization", "Bearer caller"),
        ("accept", "text/plain"),
        ("Content-Type", "text/xml"),
        ("authorization", "lower"),
    ]);
    let merged = validate_environment(&caller, None, &settings);
    assert_eq!(
        merged,
        headers_of(&[
            ("Authorization", "Bearer caller"),
            ("accept", "text/plain"),
            ("Content-Type", "text/xml"),
            ("authorization", "lower")
        ])
    );
}

#[test]
fn embedding_options_copy_only_the_supported_names() {
    let requested =
        object(r#"{"dimensions":8,"encoding_format":"float","user":"u","other":1,"prompt":null}"#);
    let mut optional = object(r#"{"existing":true,"user":"old"}"#);
    map_embedding_params(&requested, &mut optional);
    assert_eq!(
        body_value(&optional),
        expected(r#"{"existing": true, "user": "u", "dimensions": 8, "encoding_format": "float"}"#)
    );
}

#[test]
fn embedding_options_copy_an_explicit_null() {
    let requested = object(r#"{"dimensions":null}"#);
    let mut optional = Object::new();
    map_embedding_params(&requested, &mut optional);
    assert_eq!(body_value(&optional), expected(r#"{"dimensions": null}"#));
}

#[test]
fn embedding_request_lets_options_overlay_input_and_model() {
    let optional = object(r#"{"dimensions":8,"model":"over","input":"override"}"#);
    let input = Json::List(vec![Json::Text(String::from("hello"))]);
    let body = transform_embedding_request("m", &input, &optional);
    assert_eq!(
        body_value(&body),
        expected(r#"{"input": "override", "model": "over", "dimensions": 8}"#)
    );
}

fn reply(body: &str, status: u16, headers: &Headers) -> Result<CometEmbeddingReply, CometApiError> {
    decode_comet_embedding_reply(body.as_bytes(), status, headers)
}

#[test]
fn decode_embedding_reply_copies_the_nullable_fields() {
    let empty = Headers::new();
    let full = reply(
        r#"{"model":"up","data":[{"object":"embedding","embedding":[1,2],"index":0}],"object":"list","usage":{"prompt_tokens":3,"total_tokens":4}}"#,
        200,
        &empty,
    )
    .unwrap();
    assert_eq!(full.model, Some(Json::Text(String::from("up"))));
    assert_eq!(
        full.data,
        Some(Json::Object(object(
            r#"[{"object":"embedding","embedding":[1,2],"index":0}]"#
        )))
    );
    assert_eq!(full.object, Some(Json::Text(String::from("list"))));
    let nulls = reply(r#"{"model":null,"data":null,"object":null}"#, 200, &empty).unwrap();
    assert_eq!(nulls.model, None);
    assert_eq!(nulls.data, None);
    assert_eq!(nulls.object, None);
    let absent = reply("{}", 200, &empty).unwrap();
    assert_eq!(absent.model, None);
    assert_eq!(absent.data, None);
    assert_eq!(absent.object, None);
}

#[rstest]
#[case::both(r#"{"usage":{"prompt_tokens":3,"total_tokens":4}}"#, 3, 4)]
#[case::prompt_only(r#"{"usage":{"prompt_tokens":3}}"#, 3, 0)]
#[case::total_only(r#"{"usage":{"total_tokens":4}}"#, 0, 4)]
#[case::empty_usage(r#"{"usage":{}}"#, 0, 0)]
#[case::null_counts(r#"{"usage":{"prompt_tokens":null,"total_tokens":null}}"#, 0, 0)]
#[case::zero_counts(r#"{"usage":{"prompt_tokens":0,"total_tokens":0}}"#, 0, 0)]
#[case::no_usage("{}", 0, 0)]
fn decode_embedding_reply_sets_usage_counters_independently(
    #[case] body: &str,
    #[case] prompt_tokens: i64,
    #[case] total_tokens: i64,
) {
    let parsed = reply(body, 200, &Headers::new()).unwrap();
    assert_eq!(parsed.usage.prompt_tokens, prompt_tokens);
    assert_eq!(parsed.usage.total_tokens, total_tokens);
}

#[test]
fn decode_embedding_reply_wraps_json_failure_with_the_response() {
    let supplied = headers_of(&[("X-Request-Id", "r")]);
    let error = reply("not json", 502, &supplied).unwrap_err();
    assert_eq!(error.message, "not json");
    assert_eq!(error.status_code, Some(502));
    assert_eq!(error.headers, supplied);
}

#[rstest]
#[case::usage_null(r#"{"usage":null}"#)]
#[case::usage_number(r#"{"usage":5}"#)]
#[case::prompt_text(r#"{"usage":{"prompt_tokens":"x"}}"#)]
#[case::body_list("[]")]
fn decode_embedding_reply_names_a_wrong_typed_field(#[case] body: &str) {
    let error = reply(body, 200, &Headers::new()).unwrap_err();
    assert_eq!(error.status_code, None);
}

#[test]
fn decode_embedding_reply_rejects_an_oversized_body() {
    let body = vec![b'x'; MAX_REPLY_BYTES + 1];
    let error = decode_comet_embedding_reply(&body, 200, &Headers::new()).unwrap_err();
    assert_eq!(error.status_code, None);
    assert!(error.message.contains("over the"));
}

#[test]
fn decode_embedding_reply_accepts_a_body_at_the_byte_limit() {
    let mut body = String::from(r#"{"usage":{"prompt_tokens":3,"total_tokens":4}}"#).into_bytes();
    body.resize(MAX_REPLY_BYTES, b' ');
    let parsed = decode_comet_embedding_reply(&body, 200, &Headers::new()).unwrap();
    assert_eq!(parsed.usage.prompt_tokens, 3);
}

#[test]
fn embedding_error_class_keeps_the_supplied_status_and_headers() {
    let supplied = headers_of(&[("retry-after", "2")]);
    let error = get_error_class("rate limited", 429, &supplied);
    assert_eq!(error.message, "rate limited");
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.headers, supplied);
}

#[rstest]
fn replays_comet_embedding_corpus(#[files("fuzz/corpus/comet_embedding_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _reply = decode_comet_embedding_reply(&bytes, 200, &Headers::new());
}
