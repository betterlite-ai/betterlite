#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

use super::error::{CompatibleWireError, OpenAiLikeError};
use super::{
    ChatParams, EndpointType, LogInput, MISSING_BASE, MISSING_KEY, ReplyOptions,
    decode_compatible_chat_reply, header_map, system_clock, transform_response,
    validate_environment,
};
use crate::chat::response::{ChatResponse, ChatUsage};
use crate::compatible_models::decode_compatible_model_list;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

struct Sink(Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0.lock().unwrap().push(record);
    }
}

fn observe<T, A>(action: A) -> (T, LoggingState, Vec<LogRecord>)
where
    A: FnOnce(&mut LoggingState, &PrePostEnv<'_>) -> T,
{
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "trace".to_owned(),
        Duration::default,
    )
    .unwrap();
    let sink = Sink(Mutex::new(Vec::new()));
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    let result = action(&mut state, &env);
    (result, state, sink.0.into_inner().unwrap())
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn headers(items: &[(&str, &str)]) -> Headers {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(value: &Value) -> Object {
    value.as_object().unwrap().clone()
}

fn chat_reply(model: &str, usage: &str, message: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"c","object":"chat.completion","created":1,"model":{model},"choices":[{{"index":0,"message":{message},"finish_reason":"stop"}}],"usage":{usage}}}"#
    )
    .into_bytes()
}

const PLAIN: &str = r#"{"role":"assistant","content":"hi"}"#;
const USAGE: &str = r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#;
const ONE_TOOL: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}}]}"#;
const TWO_TOOLS: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"x"}},{"id":"u","type":"function","function":{"name":"g","arguments":"y"}}]}"#;
const NO_TOOLS: &str = r#"{"role":"assistant","content":null,"tool_calls":[]}"#;
const NULL_TOOLS: &str = r#"{"role":"assistant","content":"kept","tool_calls":null}"#;
const EMPTY_ARGUMENTS: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"json_tool_call","arguments":""}}]}"#;
const NULL_ARGUMENTS: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":null}}]}"#;

fn fixed_id() -> Uuid {
    Uuid::nil()
}

fn fixed_clock() -> i64 {
    1_700_000_000
}

fn options<'call>(
    json_mode: bool,
    provider: Option<&'call str>,
    base_model: Option<&'call str>,
) -> ReplyOptions<'call> {
    ReplyOptions {
        json_mode,
        provider,
        base_model,
        identifier: fixed_id,
        clock: fixed_clock,
    }
}

fn decode(bytes: &[u8], json_mode: bool) -> ChatResponse {
    decode_compatible_chat_reply(bytes, &options(json_mode, None, None))
        .unwrap()
        .response
}

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_missing_key_without_headers_is_a_400_before_the_base_is_checked() {
    let both = validate_environment(None, None, EndpointType::ChatCompletions, None, false);
    let key_only =
        validate_environment(None, Some("b"), EndpointType::ChatCompletions, None, false);
    let expected = OpenAiLikeError {
        status_code: 400,
        message: MISSING_KEY.to_owned(),
    };
    assert_eq!(both.unwrap_err(), expected);
    assert_eq!(key_only.unwrap_err(), expected);
}

#[rstest]
#[case::key_without_headers(Some("k"), None)]
#[case::empty_headers_without_key(None, Some(Headers::new()))]
fn a_missing_base_is_a_400_on_its_own(
    #[case] key_text: Option<&str>,
    #[case] explicit: Option<Headers>,
) {
    let key = key_text.map(secret);
    let error = validate_environment(
        key.as_ref(),
        None,
        EndpointType::Embeddings,
        explicit,
        false,
    )
    .unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, MISSING_BASE);
}

#[test]
fn empty_key_and_base_are_accepted() {
    let key = secret("");
    let (url, sent) = validate_environment(
        Some(&key),
        Some(""),
        EndpointType::ChatCompletions,
        None,
        false,
    )
    .unwrap();
    assert_eq!(url, "/chat/completions");
    assert_eq!(
        sent,
        headers(&[
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer ")
        ])
    );
}

#[rstest]
#[case::default_headers(Some("k"), None, &[("Content-Type", "application/json"), ("Authorization", "Bearer k")])]
#[case::empty_map_gets_no_content_type(None, Some(&[][..]), &[])]
#[case::empty_map_with_key(Some("k"), Some(&[][..]), &[("Authorization", "Bearer k")])]
#[case::other_case_is_another_header(Some("k"), Some(&[("authorization", "mine")][..]), &[("authorization", "mine"), ("Authorization", "Bearer k")])]
#[case::exact_header_wins(Some("k"), Some(&[("Authorization", "mine")][..]), &[("Authorization", "mine")])]
#[case::headers_without_key(None, Some(&[("X", "1")][..]), &[("X", "1")])]
fn headers_follow_the_key_and_the_explicit_map(
    #[case] key_text: Option<&str>,
    #[case] explicit: Option<&[(&str, &str)]>,
    #[case] expected: &[(&str, &str)],
) {
    let key = key_text.map(secret);
    let (_, sent) = validate_environment(
        key.as_ref(),
        Some("https://h"),
        EndpointType::ChatCompletions,
        explicit.map(headers),
        false,
    )
    .unwrap();
    assert_eq!(sent, headers(expected));
}

#[rstest]
#[case::chat(EndpointType::ChatCompletions, false, "https://h/chat/completions")]
#[case::embeddings(EndpointType::Embeddings, false, "https://h/embeddings")]
#[case::custom_chat(EndpointType::ChatCompletions, true, "https://h")]
#[case::custom_embeddings(EndpointType::Embeddings, true, "https://h")]
fn the_endpoint_suffix_follows_the_type_unless_custom(
    #[case] endpoint: EndpointType,
    #[case] custom_endpoint: bool,
    #[case] expected: &str,
) {
    let key = secret("k");
    let (url, _) = validate_environment(
        Some(&key),
        Some("https://h"),
        endpoint,
        None,
        custom_endpoint,
    )
    .unwrap();
    assert_eq!(url, expected);
}

#[test]
fn the_error_carries_a_synthetic_post_request() {
    let response = OpenAiLikeError {
        status_code: 418,
        message: "m".to_owned(),
    }
    .response();
    assert_eq!(response.status_code, 418);
    assert_eq!(response.request.method, "POST");
    assert_eq!(response.request.url, "https://www.litellm.ai");
}

#[test]
fn header_names_and_values_must_be_valid() {
    let map = header_map(&headers(&[("X-A", "1")])).unwrap();
    assert_eq!(map.get("x-a").unwrap(), "1");
    header_map(&headers(&[("bad name", "1")])).unwrap_err();
    header_map(&headers(&[("X-A", "line\nbreak")])).unwrap_err();
}

#[test]
fn headers_that_differ_only_in_case_both_reach_the_request() {
    let key = secret("k");
    let (_, sent) = validate_environment(
        Some(&key),
        Some("https://h"),
        EndpointType::ChatCompletions,
        Some(headers(&[("authorization", "mine")])),
        false,
    )
    .unwrap();
    let map = header_map(&sent).unwrap();
    let values = map
        .get_all("authorization")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values, ["Bearer k", "mine"]);
}

fn map_chat(replace_max_completion_tokens: bool, requested: &str) -> Value {
    let config = OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let params = ChatParams {
        config: &config,
        catalog: &catalog,
        replace_max_completion_tokens,
    };
    let mut existing = Object::new();
    params.map("m", &object(&parsed(requested)), &mut existing);
    Value::Object(existing)
}

#[rstest]
#[case::replaced(true, r#"{"max_completion_tokens":5}"#, r#"{"max_tokens":5}"#)]
#[case::kept_as_the_parent_maps_it(
    false,
    r#"{"max_completion_tokens":5}"#,
    r#"{"max_completion_tokens":5}"#
)]
#[case::nothing_to_replace(true, r#"{"max_tokens":7}"#, r#"{"max_tokens":7}"#)]
fn max_completion_tokens_is_replaced_only_when_asked(
    #[case] replace: bool,
    #[case] requested: &str,
    #[case] expected: &str,
) {
    assert_eq!(map_chat(replace, requested), parsed(expected));
}

#[test]
fn null_top_level_token_counts_become_zero_and_nothing_else_changes() {
    let usage = r#"{"prompt_tokens":null,"completion_tokens":5,"total_tokens":null,"completion_tokens_details":{"reasoning_tokens":null},"prompt_tokens_details":null}"#;
    let response = decode(&chat_reply("\"m\"", usage, PLAIN), false);
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 0);
    assert_eq!(response.usage.prompt_tokens_details, None);
    assert_eq!(
        response.usage.completion_tokens_details,
        Some(BTreeMap::from([("reasoning_tokens".to_owned(), None)]))
    );
}

#[test]
fn a_null_usage_is_read_as_the_default_usage() {
    let response = decode(&chat_reply("\"m\"", "null", PLAIN), false);
    assert_eq!(response.usage, ChatUsage::default());
}

#[test]
fn an_absent_usage_count_is_read_as_zero() {
    let response = decode(
        &chat_reply("\"m\"", r#"{"completion_tokens":5}"#, PLAIN),
        false,
    );
    let expected = ChatUsage {
        prompt_tokens: 0,
        completion_tokens: 5,
        total_tokens: 0,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    };
    assert_eq!(response.usage, expected);
}

#[rstest]
#[case::one_call_becomes_content(ONE_TOOL, true, Some("{\"a\":1}"), false)]
#[case::empty_arguments_stay_empty(EMPTY_ARGUMENTS, true, Some(""), false)]
#[case::null_arguments_become_empty(NULL_ARGUMENTS, true, Some(""), false)]
#[case::two_calls_stay(TWO_TOOLS, true, None, true)]
#[case::no_calls_stay(NO_TOOLS, true, None, true)]
#[case::null_calls_keep_content(NULL_TOOLS, true, Some("kept"), false)]
#[case::json_mode_off(ONE_TOOL, false, None, true)]
fn json_mode_converts_exactly_one_tool_call(
    #[case] message: &str,
    #[case] json_mode: bool,
    #[case] content: Option<&str>,
    #[case] tools_remain: bool,
) {
    let response = decode(&chat_reply("\"m\"", USAGE, message), json_mode);
    let delivered = &response.choices[0].message;
    assert_eq!(delivered.content.as_deref(), content);
    assert_eq!(delivered.tool_calls.is_some(), tools_remain);
}

#[rstest]
#[case::plain("\"m\"", None, "m")]
#[case::null("null", None, "")]
#[case::prefixed("\"m\"", Some("acme"), "acme/m")]
#[case::empty_provider_still_prefixes("\"m\"", Some(""), "/m")]
#[case::null_model_is_prefixed_empty("null", Some("acme"), "acme/")]
fn the_model_takes_the_provider_prefix_when_one_is_given(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: &str,
) {
    let reply = decode_compatible_chat_reply(
        &chat_reply(model, USAGE, PLAIN),
        &options(false, provider, None),
    )
    .unwrap();
    assert_eq!(reply.response.model, expected);
}

#[test]
fn the_system_clock_returns_unix_seconds() {
    assert!(system_clock() > 1_700_000_000);
}

#[test]
fn a_missing_model_reads_as_empty() {
    let bytes = br#"{"id":"c","object":"chat.completion","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;
    assert_eq!(decode(bytes, false).model, "");
}

#[rstest]
#[case::base_model(Some("base"), Some("base"))]
#[case::empty_base_model(Some(""), Some(""))]
#[case::none(None, None)]
fn the_hidden_model_comes_from_a_given_base_model(
    #[case] base_model: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let reply = decode_compatible_chat_reply(
        &chat_reply("\"m\"", USAGE, PLAIN),
        &options(false, None, base_model),
    )
    .unwrap();
    assert_eq!(reply.hidden_model.as_deref(), expected);
    assert_eq!(reply.response.model, "m");
}

const XID: &str = "chatcmpl-00000000-0000-0000-0000-000000000000";

#[test]
fn a_reply_without_id_created_or_object_gets_the_model_response_defaults() {
    let reply = decode_compatible_chat_reply(
        br#"{"model":"m","choices":[]}"#,
        &options(false, None, None),
    )
    .unwrap();
    assert_eq!(reply.response.id, XID);
    assert_eq!(reply.response.created, 1_700_000_000);
    assert_eq!(reply.response.object, "chat.completion");
}

#[test]
fn null_id_created_and_object_get_the_same_defaults() {
    let reply = decode_compatible_chat_reply(
        br#"{"id":null,"object":null,"created":null,"model":"m","choices":[]}"#,
        &options(false, None, None),
    )
    .unwrap();
    assert_eq!(reply.response.id, XID);
    assert_eq!(reply.response.created, 1_700_000_000);
    assert_eq!(reply.response.object, "chat.completion");
}

#[test]
fn a_given_id_created_and_object_keep_only_the_written_object() {
    let reply = decode_compatible_chat_reply(
        br#"{"id":"given","object":"custom","created":7,"model":"m","choices":[]}"#,
        &options(false, None, None),
    )
    .unwrap();
    assert_eq!(reply.response.id, "given");
    assert_eq!(reply.response.created, 7);
    assert_eq!(reply.response.object, "chat.completion");
}

#[rstest]
#[case::not_json(b"{", true)]
#[case::missing_choices(
    br#"{"id":"c","object":"chat.completion","created":1,"model":"m"}"#,
    false
)]
#[case::not_an_object(b"[]", false)]
fn a_reply_that_is_not_a_chat_completion_is_refused(#[case] bytes: &[u8], #[case] is_json: bool) {
    let error = decode_compatible_chat_reply(bytes, &options(false, None, None)).unwrap_err();
    assert_eq!(matches!(error, CompatibleWireError::Reply(_)), !is_json);
    assert_eq!(matches!(error, CompatibleWireError::Json(_)), is_json);
}

fn log_input() -> LogInput {
    LogInput {
        messages: Json::Text("hello".to_owned()),
        data: RawBody::Object(IndexMap::from([(
            "model".to_owned(),
            Json::Text("m".to_owned()),
        )])),
    }
}

#[test]
fn transform_response_logs_the_decoded_reply_then_cleans_it() {
    let bytes = chat_reply(
        "\"m\"",
        r#"{"prompt_tokens":null,"completion_tokens":2,"total_tokens":3}"#,
        PLAIN,
    );
    let (reply, state, records) = observe(|state, env| {
        transform_response(&bytes, &options(false, None, None), state, env, log_input()).unwrap()
    });
    assert_eq!(reply.response.usage.prompt_tokens, 0);
    assert_eq!(reply.response.model, "m");
    assert_eq!(reply.hidden_model, None);
    assert_eq!(
        records
            .iter()
            .filter(|record| record.message.starts_with("RAW RESPONSE"))
            .count(),
        1
    );
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    let logged: Value =
        serde_json::from_str(details["original_response"].as_str().unwrap()).unwrap();
    assert_eq!(logged["usage"]["prompt_tokens"], Value::Null);
    assert_eq!(details["input"], "hello");
    assert_eq!(
        details["additional_args"]["complete_input_dict"],
        parsed(r#"{"model":"m"}"#)
    );
}

#[test]
fn transform_response_logs_nothing_for_a_body_that_is_not_json() {
    let (error, _, records) = observe(|state, env| {
        transform_response(b"{", &options(false, None, None), state, env, log_input()).unwrap_err()
    });
    assert!(matches!(error, CompatibleWireError::Json(_)));
    assert!(records.is_empty());
}

#[rstest]
fn replays_the_compatible_reply_corpus(#[files("fuzz/corpus/compatible_reply/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let (tag, body) = bytes.split_first().unwrap();
    let is_chat = tag.is_multiple_of(2);
    assert_eq!(
        path.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("chat_"),
        is_chat
    );
    if is_chat {
        let json_mode = tag.is_multiple_of(4);
        let chat =
            decode_compatible_chat_reply(body, &options(json_mode, Some("fuzz"), Some("base")));
        if let Ok(reply) = chat {
            serde_json::to_vec(&reply.response).unwrap();
        }
    } else if let Ok(list) = decode_compatible_model_list(body) {
        for card in &list.data {
            let found = card.token_limits();
            if let Some(context) = found.max_tokens {
                assert!(
                    found
                        .max_input_tokens
                        .is_none_or(|tokens| tokens <= context)
                );
                assert!(
                    found
                        .max_output_tokens
                        .is_none_or(|tokens| tokens <= context)
                );
            }
        }
    } else {
        assert!(bytes.len() > 1);
    }
}
