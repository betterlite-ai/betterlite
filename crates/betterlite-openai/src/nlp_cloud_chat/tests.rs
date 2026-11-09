#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;

use super::NlpCloudConfig;
use super::error::{NlpCloudError, get_error_class};
use super::headers::validate_environment;
use super::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::reply::{ReplyInput, decode_reply};
use super::request::{CompletionInput, CompletionRequest, completion_request, transform_request};
use super::stream::{MAX_BUFFER_BYTES, StreamCleaner, StreamError, decode_stream_line};
use crate::chat::ChatMessage;
use crate::chat::response::ChatResponse;
use crate::openai_base_config::{Headers, Object};

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn messages(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).unwrap()
}

fn reply(status_code: u16, body: &str) -> Result<ChatResponse, NlpCloudError> {
    decode_reply(ReplyInput {
        status_code,
        body: body.as_bytes(),
        model: "dolphin",
        created: 1_700_000_000,
        id: String::from("reply-id"),
    })
}

fn completion(
    api_base: &str,
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    defaults: Object,
) -> CompletionRequest {
    let config = NlpCloudConfig::new(defaults).unwrap();
    let caller_headers = Headers::new();
    let input = CompletionInput {
        model,
        messages,
        api_base,
        api_key: None,
        caller_headers: &caller_headers,
        optional_params,
        config: &config,
    };
    completion_request(&input).unwrap()
}

#[test]
fn supported_params_are_the_eight_litellm_names_in_order() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_tokens",
            "stream",
            "temperature",
            "top_p",
            "presence_penalty",
            "frequency_penalty",
            "n",
            "stop",
        ]
    );
}

#[rstest]
#[case::max_tokens("max_tokens", "max_length")]
#[case::stream("stream", "stream")]
#[case::temperature("temperature", "temperature")]
#[case::top_p("top_p", "top_p")]
#[case::presence_penalty("presence_penalty", "presence_penalty")]
#[case::frequency_penalty("frequency_penalty", "frequency_penalty")]
#[case::n("n", "num_return_sequences")]
#[case::stop("stop", "stop_sequences")]
fn params_map_the_supported_names(#[case] given: &str, #[case] expected: &str) {
    let params = object(&format!(r#"{{"{given}": 1}}"#));
    let mapped = map_openai_params(&params, Object::new());
    assert_eq!(mapped.get(expected), Some(&Value::from(1_i64)));
    assert_eq!(mapped.len(), 1);
}

#[test]
fn params_keep_the_options_they_receive() {
    let mapped = map_openai_params(&object(r#"{"n": 2}"#), object(r#"{"temperature": 0.5}"#));
    assert_eq!(
        mapped,
        object(r#"{"temperature": 0.5, "num_return_sequences": 2}"#)
    );
}

#[test]
fn params_overwrite_an_option_already_mapped() {
    let mapped = map_openai_params(
        &object(r#"{"max_tokens": 5}"#),
        object(r#"{"max_length": 1}"#),
    );
    assert_eq!(mapped.get("max_length"), Some(&Value::from(5_i64)));
}

#[test]
fn params_ignore_names_outside_the_supported_list() {
    let mapped = map_openai_params(&object(r#"{"top_k": 5}"#), Object::new());
    assert!(mapped.is_empty());
}

#[test]
fn headers_always_carry_accept_and_content_type() {
    let headers = validate_environment(&Headers::new(), None);
    assert_eq!(
        headers.get("accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(headers.len(), 2);
}

#[test]
fn headers_add_the_token_scheme_for_a_key() {
    let key = SecretString::from("secret");
    let headers = validate_environment(&Headers::new(), Some(&key));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Token secret")
    );
    assert_eq!(headers.len(), 3);
}

#[test]
fn headers_use_an_empty_key_as_given() {
    let key = SecretString::from("");
    let headers = validate_environment(&Headers::new(), Some(&key));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Token ")
    );
}

#[test]
fn headers_replace_the_callers_map() {
    let caller = Headers::from([
        (String::from("accept"), String::from("text/plain")),
        (String::from("x-call"), String::from("dropped")),
    ]);
    let headers = validate_environment(&caller, None);
    assert_eq!(
        headers.get("accept").map(String::as_str),
        Some("application/json")
    );
    assert!(!headers.contains_key("x-call"));
    assert_eq!(headers.len(), 2);
}

#[rstest]
#[case::trailing_slash(
    "https://api.nlpcloud.io/v1/gpu/",
    "dolphin",
    "https://api.nlpcloud.io/v1/gpu/dolphin/generation"
)]
#[case::no_joining_slash("https://host", "dolphin", "https://hostdolphin/generation")]
#[case::model_prefix_kept(
    "https://host/",
    "nlp_cloud/dolphin",
    "https://host/nlp_cloud/dolphin/generation"
)]
fn url_appends_the_model_and_generation_without_adding_a_slash(
    #[case] api_base: &str,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let request = completion(api_base, model, &[], &Object::new(), Object::new());
    assert_eq!(request.url, expected);
}

#[test]
fn completion_request_builds_the_headers_and_merges_the_defaults() {
    let messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let optional_params = object(r#"{"max_length": null, "temperature": 0.2}"#);
    let request = completion(
        "https://host/",
        "dolphin",
        &messages,
        &optional_params,
        object(r#"{"max_length": 50, "top_k": 5}"#),
    );
    assert_eq!(request.url, "https://host/dolphin/generation");
    assert_eq!(
        request.headers,
        Headers::from([
            (String::from("accept"), String::from("application/json")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
        ])
    );
    assert_eq!(
        request.body,
        object(r#"{"text": "hi", "max_length": null, "temperature": 0.2, "top_k": 5}"#)
    );
}

#[test]
fn request_joins_message_content_with_single_spaces() {
    let messages = messages(
        r#"[{"role": "system", "content": "be brief"}, {"role": "user", "content": "hello"}]"#,
    );
    let body = transform_request(&messages, &Object::new()).unwrap();
    assert_eq!(body.get("text"), Some(&Value::from("be brief hello")));
    assert_eq!(body.len(), 1);
}

#[test]
fn request_counts_absent_content_as_an_empty_message() {
    let messages = messages(
        r#"[{"role": "user", "content": "a"}, {"role": "assistant"}, {"role": "user", "content": "b"}]"#,
    );
    let body = transform_request(&messages, &Object::new()).unwrap();
    assert_eq!(body.get("text"), Some(&Value::from("a  b")));
}

#[test]
fn request_with_no_messages_has_an_empty_text() {
    let body = transform_request(&[], &Object::new()).unwrap();
    assert_eq!(body.get("text"), Some(&Value::from("")));
}

#[test]
fn request_copies_the_options_into_the_body() {
    let body = transform_request(&[], &object(r#"{"max_length": 5, "stream": true}"#)).unwrap();
    assert_eq!(
        body,
        object(r#"{"text": "", "max_length": 5, "stream": true}"#)
    );
}

#[test]
fn request_lets_an_option_override_the_text_key() {
    let messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let body = transform_request(&messages, &object(r#"{"text": "override"}"#)).unwrap();
    assert_eq!(body.get("text"), Some(&Value::from("override")));
}

#[rstest]
#[case::parts(
    r#"[{"role": "user", "content": [{"type": "text", "text": "hi"}]}]"#,
    0
)]
#[case::second_message(
    r#"[{"role": "user", "content": "ok"}, {"role": "user", "content": [{"type": "text", "text": "hi"}]}]"#,
    1
)]
fn request_rejects_message_content_that_is_not_a_plain_string(
    #[case] json: &str,
    #[case] index: usize,
) {
    let error = transform_request(&messages(json), &Object::new()).unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(
        error.message,
        format!("message {index} content is not a plain string")
    );
    assert_eq!(error.headers, None);
}

#[test]
fn config_rejects_an_unknown_setting() {
    let error = NlpCloudConfig::new(object(r#"{"top_k": 5, "max_new_tokens": 5}"#)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "NLP Cloud has no max_new_tokens config setting"
    );
}

#[test]
fn config_drops_null_defaults() {
    let config = NlpCloudConfig::new(object(r#"{"max_length": null, "top_k": 5}"#)).unwrap();
    let mut options = Object::new();
    config.merge_defaults(&mut options);
    assert_eq!(options, object(r#"{"top_k": 5}"#));
}

#[test]
fn defaults_fill_only_the_absent_options() {
    let config = NlpCloudConfig::new(object(
        r#"{"max_length": 50, "temperature": 0.7, "top_k": 5}"#,
    ))
    .unwrap();
    let mut options = object(r#"{"max_length": 20, "temperature": 0.2, "top_k": null}"#);
    config.merge_defaults(&mut options);
    assert_eq!(
        options,
        object(r#"{"max_length": 20, "temperature": 0.2, "top_k": null}"#)
    );
}

#[test]
fn defaults_do_not_replace_explicit_false_or_empty_options() {
    let config =
        NlpCloudConfig::new(object(r#"{"remove_input": true, "end_sequence": "END"}"#)).unwrap();
    let mut options = object(r#"{"remove_input": false, "end_sequence": ""}"#);
    config.merge_defaults(&mut options);
    assert_eq!(
        options,
        object(r#"{"remove_input": false, "end_sequence": ""}"#)
    );
}

#[test]
fn defaults_keep_explicit_false_and_empty_values() {
    let config =
        NlpCloudConfig::new(object(r#"{"remove_input": false, "end_sequence": ""}"#)).unwrap();
    let mut options = Object::new();
    config.merge_defaults(&mut options);
    assert_eq!(
        options,
        object(r#"{"remove_input": false, "end_sequence": ""}"#)
    );
}

#[test]
fn error_class_carries_the_status_message_and_headers() {
    let headers = Headers::from([(String::from("retry-after"), String::from("2"))]);
    let error = get_error_class("slow down", 429, Some(headers.clone()));
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(error.headers, Some(headers));
}

#[test]
fn reply_maps_generated_text_usage_and_finish() {
    let response = reply(
        200,
        r#"{"generated_text": "hello", "nb_input_tokens": 3, "nb_generated_tokens": 4}"#,
    )
    .unwrap();
    assert_eq!(response.id, "reply-id");
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.model, "dolphin");
    assert_eq!(response.system_fingerprint, None);
    assert_eq!(response.service_tier, None);
    assert_eq!(response.usage.prompt_tokens, 3);
    assert_eq!(response.usage.completion_tokens, 4);
    assert_eq!(response.usage.total_tokens, 7);
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.index, 0);
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("hello"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn reply_leaves_an_empty_generated_text_null() {
    let response = reply(
        200,
        r#"{"generated_text": "", "nb_input_tokens": 1, "nb_generated_tokens": 0}"#,
    )
    .unwrap();
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.message.content, None);
    assert_eq!(response.usage.total_tokens, 1);
}

#[test]
fn reply_ignores_extra_keys() {
    let response = reply(
        200,
        r#"{"generated_text": "hi", "nb_input_tokens": 1, "nb_generated_tokens": 1, "details": {}}"#,
    )
    .unwrap();
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.message.content.as_deref(), Some("hi"));
}

#[rstest]
#[case::not_json("not json at all", 500)]
#[case::empty("", 200)]
#[case::array("[]", 200)]
#[case::missing_generated_text(r#"{"nb_input_tokens": 1, "nb_generated_tokens": 2}"#, 200)]
#[case::null_generated_text(
    r#"{"generated_text": null, "nb_input_tokens": 1, "nb_generated_tokens": 2}"#,
    200
)]
#[case::numeric_generated_text(
    r#"{"generated_text": 5, "nb_input_tokens": 1, "nb_generated_tokens": 2}"#,
    200
)]
fn reply_rejects_a_body_without_generated_text(#[case] body: &str, #[case] status: u16) {
    let error = reply(status, body).unwrap_err();
    assert_eq!(error.status_code, status);
    assert_eq!(error.message, body);
    assert_eq!(error.headers, None);
}

#[test]
fn reply_error_key_wins_over_generated_text() {
    let error = reply(
        401,
        r#"{"error": "bad key", "generated_text": "hi", "nb_input_tokens": 1, "nb_generated_tokens": 1}"#,
    )
    .unwrap_err();
    assert_eq!(error.status_code, 401);
    assert_eq!(error.message, "bad key");
}

#[test]
fn reply_non_text_error_value_is_written_as_json() {
    let error = reply(400, r#"{"error": {"code": "x"}}"#).unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, r#"{"code":"x"}"#);
}

#[rstest]
#[case::no_usage(r#"{"generated_text": "hi"}"#)]
#[case::missing_completion(r#"{"generated_text": "hi", "nb_input_tokens": 1}"#)]
#[case::text_counter(
    r#"{"generated_text": "hi", "nb_input_tokens": "1", "nb_generated_tokens": 2}"#
)]
#[case::negative_counter(
    r#"{"generated_text": "hi", "nb_input_tokens": -1, "nb_generated_tokens": 2}"#
)]
#[case::fractional_counter(
    r#"{"generated_text": "hi", "nb_input_tokens": 1.5, "nb_generated_tokens": 2}"#
)]
fn reply_rejects_a_missing_or_wrong_usage_counter(#[case] body: &str) {
    let error = reply(200, body).unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn reply_rejects_a_usage_sum_that_overflows() {
    let error = reply(
        200,
        r#"{"generated_text": "hi", "nb_input_tokens": 18446744073709551615, "nb_generated_tokens": 1}"#,
    )
    .unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn reply_rejects_a_non_utf8_body_without_panicking() {
    let error = decode_reply(ReplyInput {
        status_code: 200,
        body: &[0xff, 0xfe],
        model: "dolphin",
        created: 0,
        id: String::from("reply-id"),
    })
    .unwrap_err();
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "the reply body is not valid UTF-8");
}

#[test]
fn cleaner_removes_every_nul_byte_from_a_line() {
    let mut cleaner = StreamCleaner::default();
    assert_eq!(
        cleaner.push("a\u{0}b\u{0}c").unwrap(),
        Some(String::from("abc"))
    );
    assert_eq!(cleaner.finish(), None);
}

#[test]
fn cleaner_flushes_the_buffer_at_each_nul_line() {
    let mut cleaner = StreamCleaner::default();
    assert_eq!(cleaner.push("one\u{0}").unwrap(), Some(String::from("one")));
    assert_eq!(cleaner.push("two").unwrap(), None);
    assert_eq!(cleaner.finish(), Some(String::from("two")));
}

#[test]
fn cleaner_keeps_lines_without_nuls_until_the_stream_ends() {
    let mut cleaner = StreamCleaner::default();
    assert_eq!(cleaner.push("plain").unwrap(), None);
    assert_eq!(cleaner.push("text").unwrap(), None);
    assert_eq!(cleaner.finish(), Some(String::from("plaintext")));
}

#[test]
fn cleaner_skips_a_line_that_is_only_nuls() {
    let mut cleaner = StreamCleaner::default();
    assert_eq!(cleaner.push("\u{0}").unwrap(), None);
    assert_eq!(cleaner.push("tail").unwrap(), None);
    assert_eq!(cleaner.finish(), Some(String::from("tail")));
}

#[test]
fn cleaner_skips_empty_lines_and_keeps_the_rest() {
    let mut cleaner = StreamCleaner::default();
    assert_eq!(cleaner.push("a").unwrap(), None);
    assert_eq!(cleaner.push("").unwrap(), None);
    assert_eq!(cleaner.push("b").unwrap(), None);
    assert_eq!(cleaner.finish(), Some(String::from("ab")));
}

#[test]
fn cleaner_returns_nothing_for_no_lines_or_only_empty_lines() {
    let mut empty = StreamCleaner::default();
    assert_eq!(empty.finish(), None);
    let mut blank = StreamCleaner::default();
    assert_eq!(blank.push("").unwrap(), None);
    assert_eq!(blank.push("").unwrap(), None);
    assert_eq!(blank.finish(), None);
}

#[test]
fn cleaner_does_not_trim_spaces() {
    let mut cleaner = StreamCleaner::default();
    assert_eq!(cleaner.push(" a ").unwrap(), None);
    assert_eq!(cleaner.finish(), Some(String::from(" a ")));
}

#[test]
fn cleaner_yields_the_first_chunk_before_later_lines_are_read() {
    let mut source = ["first\u{0}", "second", "third"]
        .into_iter()
        .inspect(|&line| assert_ne!(line, "third", "an eager cleaner reads past the first chunk"));
    let mut cleaner = StreamCleaner::default();
    let first = source.next().and_then(|line| cleaner.push(line).unwrap());
    assert_eq!(first, Some(String::from("first")));
}

#[test]
fn cleaner_rejects_a_buffer_that_crosses_the_byte_limit() {
    let mut cleaner = StreamCleaner::default();
    let full = "x".repeat(MAX_BUFFER_BYTES);
    assert_eq!(cleaner.push(&full).unwrap(), None);
    assert!(matches!(cleaner.push("y"), Err(StreamError::TooLarge)));
}

#[test]
fn stream_decoder_reads_the_generated_text() {
    let event = decode_stream_line(r#"{"generated_text": "the whole delta"}"#).unwrap();
    assert_eq!(event.generated_text, "the whole delta");
}

#[test]
fn stream_decoder_ignores_extra_keys() {
    let event = decode_stream_line(r#"{"generated_text": "hi", "details": {"x": 1}}"#).unwrap();
    assert_eq!(event.generated_text, "hi");
}

#[rstest]
#[case::not_json("not json")]
#[case::empty("")]
#[case::missing_generated_text("{}")]
#[case::null_generated_text(r#"{"generated_text": null}"#)]
#[case::numeric_generated_text(r#"{"generated_text": 5}"#)]
#[case::array("[]")]
fn stream_decoder_rejects_a_line_without_generated_text(#[case] line: &str) {
    let error = decode_stream_line(line).unwrap_err();
    assert!(matches!(error, StreamError::Malformed { .. }));
}

#[rstest]
fn replays_the_nlp_cloud_reply_corpus(#[files("fuzz/corpus/nlp_cloud_reply/*")] path: PathBuf) {
    let body = fs::read(path).unwrap();
    let _decoded = decode_reply(ReplyInput {
        status_code: 200,
        body: &body,
        model: "corpus",
        created: 0,
        id: String::from("corpus"),
    });
}

#[rstest]
fn replays_the_nlp_cloud_stream_lines_corpus(
    #[files("fuzz/corpus/nlp_cloud_stream_lines/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    let mut cleaner = StreamCleaner::default();
    let mut cleaned: Vec<String> = text
        .split('\n')
        .filter_map(|line| cleaner.push(line).unwrap())
        .collect();
    if let Some(remainder) = cleaner.finish() {
        cleaned.push(remainder);
    }
    let _decoded: Vec<_> = cleaned
        .iter()
        .map(String::as_str)
        .map(decode_stream_line)
        .collect();
}
