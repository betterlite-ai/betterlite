#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_prompts::arguments::json::{Json, Object};
use http::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use secrecy::SecretString;

use super::API_BASE;
use super::environment::{complete_url, validate_environment};
use super::error::{BytezConfigError, BytezError, get_error_class};
use super::messages::{
    NormalizedBlock, NormalizedMessage, RawMessage, get_tokens_from_messages, normalize_messages,
};
use super::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::reply::{ReplyInput, decode_bytez_reply};
use super::request::request_body;
use crate::error::ProviderError;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json")
}

fn object(text: &str) -> Object {
    json(text).as_object().expect("object").clone()
}

fn message(text: &str) -> RawMessage {
    serde_json::from_str(text).expect("message")
}

#[test]
fn supported_params_lists_only_the_renamed_entries() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "top_p",
            "n",
            "max_retries"
        ]
    );
}

#[test]
fn map_openai_params_renames_both_token_limits_and_n() {
    let mapped = map_openai_params(
        &object(r#"{"max_tokens":10,"max_completion_tokens":20,"n":2,"temperature":0.5,"top_p":0.1,"stream":true,"max_retries":3}"#),
        Object::new(),
        false,
    )
    .expect("mapped");
    assert_eq!(
        mapped,
        object(
            r#"{"max_new_tokens":20,"num_return_sequences":2,"temperature":0.5,"top_p":0.1,"stream":true,"max_retries":3}"#
        )
    );
}

#[test]
fn map_openai_params_lets_optional_params_win_and_keeps_unknown_keys() {
    let mapped = map_openai_params(
        &object(r#"{"max_tokens":10,"unknown":1,"favor": "call"}"#),
        object(r#"{"max_tokens":99,"extra": "kept"}"#),
        false,
    )
    .expect("mapped");
    assert_eq!(
        mapped,
        object(r#"{"max_new_tokens":99,"unknown":1,"favor":"call","extra":"kept"}"#)
    );
}

#[rstest]
#[case::seed("seed")]
#[case::stop("stop")]
#[case::logit_bias("logit_bias")]
#[case::logprobs("logprobs")]
#[case::frequency_penalty("frequency_penalty")]
#[case::presence_penalty("presence_penalty")]
#[case::top_logprobs("top_logprobs")]
#[case::modalities("modalities")]
#[case::prediction("prediction")]
#[case::stream_options("stream_options")]
#[case::tools("tools")]
#[case::tool_choice("tool_choice")]
#[case::function_call("function_call")]
#[case::functions("functions")]
#[case::extra_headers("extra_headers")]
#[case::parallel_tool_calls("parallel_tool_calls")]
#[case::audio("audio")]
#[case::web_search_options("web_search_options")]
fn map_openai_params_rejects_an_unsupported_param(#[case] key: &str) {
    let mut requested = Object::new();
    requested.insert(key.to_owned(), Json::Bool(true));
    assert!(matches!(
        map_openai_params(&requested, Object::new(), false),
        Err(BytezConfigError::UnsupportedParam { key: reported }) if reported == key
    ));
    let dropped = map_openai_params(&requested, Object::new(), true).expect("dropped");
    assert!(dropped.is_empty());
}

#[test]
fn map_openai_params_reports_the_first_unsupported_key_in_merge_order() {
    let error = map_openai_params(
        &object(r#"{"temperature":1,"stop":["x"]}"#),
        object(r#"{"seed":1}"#),
        false,
    )
    .expect_err("unsupported");
    assert!(matches!(error, BytezConfigError::UnsupportedParam { key } if key == "stop"));
}

#[test]
fn validate_environment_overwrites_the_identity_headers() {
    let caller = BTreeMap::from([("Authorization".to_owned(), String::from("Bearer old"))]);
    let key = SecretString::from("secret");
    let validated = validate_environment(
        caller,
        &[message(r#"{"role":"user","content":"hi"}"#)],
        Some(&key),
    )
    .expect("validated");
    assert_eq!(validated["content-type"], "application/json");
    assert_eq!(validated["Authorization"], "Key secret");
    assert_eq!(validated["user-agent"], "litellm/1.105.1");
}

#[test]
fn validate_environment_rejects_empty_messages_before_a_missing_key() {
    assert!(matches!(
        validate_environment(BTreeMap::new(), &[], None),
        Err(BytezConfigError::MissingMessages)
    ));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn validate_environment_rejects_a_missing_or_empty_key(#[case] key: Option<&str>) {
    let secret = key.map(SecretString::from);
    assert!(matches!(
        validate_environment(
            BTreeMap::new(),
            &[message(r#"{"role":"user","content":"hi"}"#)],
            secret.as_ref()
        ),
        Err(BytezConfigError::MissingApiKey)
    ));
}

#[rstest]
#[case::plain("model", "https://api.bytez.com/models/v2/model")]
#[case::segments("org/model", "https://api.bytez.com/models/v2/org/model")]
#[case::escaped("a b?c", "https://api.bytez.com/models/v2/a%20b%3Fc")]
fn complete_url_builds_on_the_fixed_api_base(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(complete_url(model).expect("url"), expected);
    assert!(expected.starts_with(API_BASE));
}

#[rstest]
#[case::empty("")]
#[case::leading_slash("/model")]
#[case::trailing_slash("model/")]
#[case::dot(".")]
#[case::dot_segment("org/..")]
fn complete_url_rejects_a_model_outside_its_segments(#[case] model: &str) {
    assert!(matches!(complete_url(model), Err(BytezConfigError::Url(_))));
}

#[rstest]
#[case::text(r#"[{"role":"user","content":"hi"}]"#, "text", "text", "hi")]
#[case::object(
    r#"[{"role":"user","content":{"type":"text","text":"hi"}}]"#,
    "text",
    "text",
    "hi"
)]
#[case::list_of_strings(r#"[{"role":"user","content":["hi"]}]"#, "text", "text", "hi")]
#[case::image(
    r#"[{"role":"user","content":{"type":"image_url","url":"https://img.test/a.png"}}]"#,
    "image",
    "url",
    "https://img.test/a.png"
)]
#[case::audio(
    r#"[{"role":"user","content":{"type":"input_audio","url":"https://a.test/a.wav"}}]"#,
    "audio",
    "url",
    "https://a.test/a.wav"
)]
#[case::video(
    r#"[{"role":"user","content":{"type":"video_url","url":"https://v.test/a.mp4"}}]"#,
    "video",
    "url",
    "https://v.test/a.mp4"
)]
fn adapt_messages_normalizes_content_shapes(
    #[case] source: &str,
    #[case] kind: &str,
    #[case] payload: &str,
    #[case] value: &str,
) {
    let messages: Vec<RawMessage> = serde_json::from_str(source).expect("messages");
    let normalized = normalize_messages(&messages).expect("normalized");
    let encoded = serde_json::to_string(&normalized[0].content).expect("encoded");
    let expected = format!(r#"[{{"type":"{kind}","{payload}":"{value}"}}]"#);
    assert_eq!(encoded, expected);
    assert_eq!(normalized[0].role.as_deref(), Some("user"));
}

#[test]
fn adapt_messages_discards_extra_message_fields_and_keeps_an_absent_role() {
    let messages = [message(r#"{"content":"hi","name":"nick","tool_calls":[]}"#)];
    let normalized = normalize_messages(&messages).expect("normalized");
    assert_eq!(normalized[0].role, None);
    let encoded = serde_json::to_string(&normalized[0]).expect("encoded");
    assert_eq!(
        encoded,
        r#"{"role":null,"content":[{"type":"text","text":"hi"}]}"#
    );
}

#[rstest]
#[case::missing_content(r#"[{"role":"user"}]"#)]
#[case::null_content(r#"[{"role":"user","content":null}]"#)]
#[case::empty_text(r#"[{"role":"user","content":""}]"#)]
#[case::number_content(r#"[{"role":"user","content":5}]"#)]
#[case::mixed_list(r#"[{"role":"user","content":["hi",5]}]"#)]
#[case::missing_type(r#"[{"role":"user","content":{"text":"hi"}}]"#)]
#[case::empty_type(r#"[{"role":"user","content":{"type":"","text":"hi"}}]"#)]
#[case::empty_payload(r#"[{"role":"user","content":{"type":"text","text":""}}]"#)]
#[case::numeric_payload(r#"[{"role":"user","content":{"type":"text","text":5}}]"#)]
#[case::document(r#"[{"role":"user","content":{"type":"document","url":"https://d.test/a.pdf"}}]"#)]
#[case::file(r#"[{"role":"user","content":{"type":"file","url":"https://d.test/a.pdf"}}]"#)]
#[case::unknown(r#"[{"role":"user","content":{"type":"mystery","url":"x"}}]"#)]
fn adapt_messages_rejects_unusable_content(#[case] source: &str) {
    let messages: Vec<RawMessage> = serde_json::from_str(source).expect("messages");
    normalize_messages(&messages).expect_err("rejected");
}

#[rstest]
#[case::document("document")]
#[case::file("file")]
fn adapt_messages_names_a_document_type_as_unsupported(#[case] kind: &str) {
    let source =
        format!(r#"[{{"role":"user","content":{{"type":"{kind}","url":"https://d.test/a"}}}}]"#);
    let messages: Vec<RawMessage> = serde_json::from_str(&source).expect("messages");
    let error = normalize_messages(&messages).expect_err("rejected");
    assert!(matches!(
        &error,
        BytezConfigError::UnsupportedContentType { kind: named } if named == kind
    ));
}

#[test]
fn request_body_writes_messages_stream_and_params() {
    let body = request_body(
        &[message(r#"{"role":"user","content":"hi"}"#)],
        &object(r#"{"stream":true,"max_new_tokens":10}"#),
    )
    .expect("body");
    assert_eq!(body.stream, Json::Bool(true));
    assert_eq!(body.params, object(r#"{"max_new_tokens":10}"#));
    let encoded = serde_json::to_string(&body).expect("encoded");
    assert_eq!(
        encoded,
        r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}],"stream":true,"params":{"max_new_tokens":10}}"#
    );
}

#[rstest]
#[case::absent("{}", "false", "{}")]
#[case::explicit_false(r#"{"stream":false}"#, "false", r#"{"stream":false}"#)]
#[case::explicit_null(r#"{"stream":null}"#, "null", r#"{"stream":null}"#)]
fn request_body_keeps_a_non_truthy_stream_in_params(
    #[case] params: &str,
    #[case] stream: &str,
    #[case] expected: &str,
) {
    let body = request_body(
        &[message(r#"{"role":"user","content":"hi"}"#)],
        &object(params),
    )
    .expect("body");
    assert_eq!(serde_json::to_string(&body.stream).expect("stream"), stream);
    assert_eq!(body.params, object(expected));
}

#[test]
fn token_count_splits_text_on_single_spaces() {
    let messages = vec![NormalizedMessage {
        role: None,
        content: vec![
            NormalizedBlock::Text {
                text: String::from("hello  world"),
            },
            NormalizedBlock::Image {
                url: String::from("https://img.test/a.png"),
            },
        ],
    }];
    assert_eq!(get_tokens_from_messages(&messages).expect("count"), 4);
}

#[test]
fn token_count_of_empty_text_is_one_word() {
    let messages = vec![NormalizedMessage {
        role: None,
        content: vec![NormalizedBlock::Text {
            text: String::new(),
        }],
    }];
    assert_eq!(get_tokens_from_messages(&messages).expect("count"), 1);
}

fn reply_input<'request>(
    body: &'request str,
    status: u16,
    messages: &'request [RawMessage],
    headers: &'request HeaderMap,
) -> ReplyInput<'request> {
    ReplyInput {
        body: body.as_bytes(),
        status,
        model: "org/model",
        created: 1_700_000_000,
        messages,
        headers,
    }
}

#[test]
fn decode_bytez_reply_reads_the_first_text_block_and_usage() {
    let messages = [message(r#"{"role":"user","content":"hello  world"}"#)];
    let reply = decode_bytez_reply(&reply_input(
        r#"{"output":{"role":"assistant","content":[{"type":"text","text":"one two"}]}}"#,
        200,
        &messages,
        &HeaderMap::new(),
    ))
    .expect("reply");
    assert_eq!(reply.content, "one two");
    assert_eq!(reply.model, "org/model");
    assert_eq!(reply.created, 1_700_000_000);
    assert_eq!(
        reply.usage,
        super::reply::BytezUsage {
            prompt_tokens: 3,
            completion_tokens: 2,
            total_tokens: 5
        }
    );
}

#[rstest]
#[case::boolean_false(r#"{"error":false}"#)]
#[case::empty_string(r#"{"error":""}"#)]
#[case::empty_object(r#"{"error":{}}"#)]
#[case::text(r#"{"error":"boom"}"#)]
fn decode_bytez_reply_raises_on_any_non_null_error(#[case] body: &str) {
    let error =
        decode_bytez_reply(&reply_input(body, 429, &[], &HeaderMap::new())).expect_err("rejected");
    assert_eq!(error.status_code(), Some(429));
}

#[test]
fn decode_bytez_reply_accepts_a_null_error() {
    let reply = decode_bytez_reply(&reply_input(
        r#"{"error":null,"output":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#,
        200,
        &[],
        &HeaderMap::new(),
    ))
    .expect("reply");
    assert_eq!(reply.content, "ok");
    assert_eq!(reply.usage.prompt_tokens, 0);
}

#[test]
fn decode_bytez_reply_attaches_every_header_and_the_rate_limit_fields() {
    let response_headers = raw_headers(&[
        ("x-request-id", b"abc"),
        ("ratelimit-limit", b"10"),
        ("ratelimit-remaining", b"9"),
        ("ratelimit-reset", b"1"),
        ("inference-meter", b"3"),
        ("inference-time", b"42ms"),
    ]);
    let reply = decode_bytez_reply(&reply_input(
        r#"{"output":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#,
        200,
        &[],
        &response_headers,
    ))
    .expect("reply");
    assert_eq!(reply.additional_headers["x-request-id"], "abc");
    assert_eq!(reply.additional_headers["ratelimit-limit"], "10");
    assert_eq!(
        reply.provider_specific_fields,
        super::reply::RateLimitFields {
            ratelimit_limit: Some(String::from("10")),
            ratelimit_remaining: Some(String::from("9")),
            ratelimit_reset: Some(String::from("1")),
            inference_meter: Some(String::from("3")),
            inference_time: Some(String::from("42ms")),
        }
    );
}

#[test]
fn decode_bytez_reply_leaves_absent_rate_limit_fields_null() {
    let reply = decode_bytez_reply(&reply_input(
        r#"{"output":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#,
        200,
        &[],
        &HeaderMap::new(),
    ))
    .expect("reply");
    let encoded = serde_json::to_string(&reply.provider_specific_fields).expect("encoded");
    assert_eq!(
        encoded,
        r#"{"ratelimit-limit":null,"ratelimit-remaining":null,"ratelimit-reset":null,"inference-meter":null,"inference-time":null}"#
    );
}

const OK_BODY: &str = r#"{"output":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#;

fn raw_headers(pairs: &[(&str, &[u8])]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).expect("name"),
            HeaderValue::from_bytes(value).expect("header"),
        );
    }
    headers
}

#[test]
fn decode_bytez_reply_reads_a_present_but_empty_header_as_empty_text() {
    let response_headers = raw_headers(&[("inference-time", b""), ("x-empty", b"")]);
    let reply =
        decode_bytez_reply(&reply_input(OK_BODY, 200, &[], &response_headers)).expect("reply");
    assert_eq!(
        reply.provider_specific_fields.inference_time,
        Some(String::new())
    );
    assert_eq!(reply.provider_specific_fields.inference_meter, None);
    assert_eq!(reply.additional_headers["x-empty"], "");
}

#[test]
fn decode_bytez_reply_decodes_non_ascii_header_values_as_utf8() {
    let response_headers = raw_headers(&[
        ("x-note", "\u{e9}t\u{e9}".as_bytes()),
        ("inference-time", b"2"),
    ]);
    let reply =
        decode_bytez_reply(&reply_input(OK_BODY, 200, &[], &response_headers)).expect("reply");
    assert_eq!(reply.additional_headers["x-note"], "\u{e9}t\u{e9}");
    assert_eq!(
        reply.provider_specific_fields.inference_time,
        Some(String::from("2"))
    );
}

#[test]
fn decode_bytez_reply_falls_back_to_latin_1_for_the_whole_header_set() {
    let response_headers = raw_headers(&[
        ("x-latin", &[0x63, 0xe9]),
        ("x-utf8", &[0xc3, 0xa9]),
        ("inference-meter", &[0xe9]),
    ]);
    let reply =
        decode_bytez_reply(&reply_input(OK_BODY, 200, &[], &response_headers)).expect("reply");
    assert_eq!(reply.additional_headers["x-latin"], "c\u{e9}");
    assert_eq!(reply.additional_headers["x-utf8"], "\u{c3}\u{a9}");
    assert_eq!(
        reply.provider_specific_fields.inference_meter,
        Some(String::from("\u{e9}"))
    );
}

#[rstest]
#[case::not_json("{")]
#[case::list_root("[1,2]")]
#[case::list_root_with_an_error_position(r#"["boom"]"#)]
#[case::list_root_with_an_output_position(
    r#"[null,{"role":"assistant","content":[{"type":"text","text":"x"}]}]"#
)]
#[case::missing_output(r#"{"error":null}"#)]
#[case::output_not_object(r#"{"output":"text"}"#)]
#[case::content_not_list(r#"{"output":{"role":"assistant","content":"text"}}"#)]
#[case::empty_content(r#"{"output":{"role":"assistant","content":[]}}"#)]
#[case::content_without_text(
    r#"{"output":{"role":"assistant","content":[{"type":"image_url","url":"x"}]}}"#
)]
fn decode_bytez_reply_rejects_a_malformed_output(#[case] body: &str) {
    let error =
        decode_bytez_reply(&reply_input(body, 200, &[], &HeaderMap::new())).expect_err("rejected");
    assert_eq!(error.status_code(), None);
}

#[test]
fn decode_bytez_reply_accepts_a_body_of_exactly_the_byte_limit() {
    let padding = " ".repeat(super::reply::MAX_REPLY_BYTES - OK_BODY.len());
    let body = format!("{OK_BODY}{padding}");
    assert_eq!(body.len(), super::reply::MAX_REPLY_BYTES);
    let reply =
        decode_bytez_reply(&reply_input(&body, 200, &[], &HeaderMap::new())).expect("reply");
    assert_eq!(reply.content, "ok");
}

#[test]
fn decode_bytez_reply_rejects_a_body_over_the_byte_limit() {
    let oversized = " ".repeat(super::reply::MAX_REPLY_BYTES + 1);
    let error = decode_bytez_reply(&reply_input(&oversized, 200, &[], &HeaderMap::new()))
        .expect_err("rejected");
    assert!(matches!(
        error,
        super::reply::BytezReplyError::TooLarge { .. }
    ));
    assert_eq!(
        super::reply::MAX_REPLY_BYTES,
        TransportLimits::DEFAULT.body_bytes
    );
}

#[test]
fn get_error_class_keeps_the_status_and_drops_headers() {
    let error = get_error_class(
        "boom",
        400,
        &BTreeMap::from([(String::from("x-test"), String::from("1"))]),
    );
    assert_eq!(
        error,
        BytezError {
            status_code: 400,
            message: String::from("boom")
        }
    );
    let provider = ProviderError::from(error);
    assert_eq!(provider.status_code, 400);
    assert_eq!(provider.message, "boom");
    assert_eq!(provider.headers, None);
    assert_eq!(provider.request.url, API_BASE);
    assert_eq!(provider.request.method, "POST");
    assert_eq!(provider.response.status_code, 400);
}

#[rstest]
fn replays_bytez_reply_corpus(#[files("fuzz/corpus/bytez_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    let _reply = decode_bytez_reply(&ReplyInput {
        body: &bytes,
        status: 200,
        model: "org/model",
        created: 1,
        messages: &[],
        headers: &HeaderMap::new(),
    });
}
