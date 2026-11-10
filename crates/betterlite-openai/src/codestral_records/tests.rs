#![cfg(test)]
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::chunk::{CodestralChunk, decode_codestral_chunk};
use super::error::{NativeError, NativeRequest, NativeResponse, TextCompletionCodestralError};
use super::options::{CodestralSettings, SUPPORTED_PARAMS, map_params};
use super::reply::{
    CodestralReply, CodestralUsage, PostCall, PostCallLogger, decode_codestral_reply,
    process_response,
};
use super::{MAX_RECORD_BYTES, RecordSource};
use crate::base_helpers::provider_config;

fn source() -> RecordSource {
    RecordSource {
        id: Uuid::nil(),
        created: 0,
    }
}

fn value(raw: &str) -> Value {
    serde_json::from_str(raw).expect("a JSON fixture")
}

fn map(raw: &str) -> Map<String, Value> {
    serde_json::from_str(raw).expect("a JSON object fixture")
}

fn reply(raw: &str) -> CodestralReply {
    decode_codestral_reply(200, raw.as_bytes(), &source()).expect("a codestral reply")
}

struct Recorder {
    calls: Vec<(String, String, Value)>,
}

impl PostCallLogger for Recorder {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.calls.push((
            call.api_key.to_owned(),
            call.original_response.to_owned(),
            call.request_data.clone(),
        ));
    }
}

#[test]
fn supported_params_are_the_codestral_list() {
    assert_eq!(
        SUPPORTED_PARAMS,
        [
            "suffix",
            "temperature",
            "top_p",
            "max_tokens",
            "max_completion_tokens",
            "stream",
            "seed",
            "stop",
        ]
    );
}

#[test]
fn settings_default_to_absent() {
    let config = provider_config(&CodestralSettings {
        suffix: None,
        temperature: None,
        top_p: None,
        max_tokens: None,
        min_tokens: None,
        stream: None,
        random_seed: None,
        stop: None,
    })
    .expect("a config object");
    assert!(config.is_empty());
}

#[test]
fn configured_settings_export_every_supplied_key() {
    let config = provider_config(&CodestralSettings {
        suffix: Some(value(r#""}""#)),
        temperature: Some(value("0.5")),
        top_p: Some(value("0.9")),
        max_tokens: Some(value("10")),
        min_tokens: Some(value("2")),
        stream: Some(true),
        random_seed: Some(value("7")),
        stop: Some(value(r#"["END"]"#)),
    })
    .expect("a config object");
    assert_eq!(config.len(), 8);
    assert_eq!(config.get("top_p"), Some(&value("0.9")));
    assert_eq!(config.get("random_seed"), Some(&value("7")));
}

#[test]
fn settings_export_only_the_supplied_keys() {
    let config = provider_config(&CodestralSettings {
        suffix: None,
        temperature: None,
        top_p: None,
        max_tokens: Some(value("4")),
        min_tokens: None,
        stream: Some(false),
        random_seed: None,
        stop: None,
    })
    .expect("a config object");
    assert_eq!(config, map(r#"{"max_tokens":4,"stream":false}"#));
}

#[rstest]
#[case::suffix("suffix", r#""}""#, "suffix")]
#[case::temperature("temperature", "0.4", "temperature")]
#[case::top_p("top_p", "0.8", "top_p")]
#[case::max_tokens("max_tokens", "5", "max_tokens")]
#[case::max_completion_tokens("max_completion_tokens", "9", "max_tokens")]
#[case::stop("stop", r#"["END"]"#, "stop")]
#[case::seed("seed", "11", "random_seed")]
#[case::min_tokens("min_tokens", "2", "min_tokens")]
fn maps_one_param(#[case] name: &str, #[case] raw: &str, #[case] target: &str) {
    let mapped = value(raw);
    let mut optional = Map::new();
    map_params(&[(name.to_owned(), mapped.clone())], &mut optional);
    assert_eq!(optional.get(target), Some(&mapped));
}

#[test]
fn max_token_spellings_overwrite_in_encounter_order() {
    let mut completion_last = Map::new();
    map_params(
        &[
            ("max_tokens".to_owned(), value("5")),
            ("max_completion_tokens".to_owned(), value("9")),
        ],
        &mut completion_last,
    );
    assert_eq!(completion_last, map(r#"{"max_tokens":9}"#));
    let mut tokens_last = Map::new();
    map_params(
        &[
            ("max_completion_tokens".to_owned(), value("9")),
            ("max_tokens".to_owned(), value("5")),
        ],
        &mut tokens_last,
    );
    assert_eq!(tokens_last, map(r#"{"max_tokens":5}"#));
}

#[rstest]
#[case::true_passes("true", Some("true"))]
#[case::false_is_skipped("false", None)]
#[case::one_is_not_true("1", None)]
fn stream_passes_only_for_the_bool(#[case] raw: &str, #[case] expected: Option<&str>) {
    let mut optional = Map::new();
    map_params(&[("stream".to_owned(), value(raw))], &mut optional);
    assert_eq!(optional.get("stream"), expected.map(value).as_ref());
}

#[test]
fn mapping_keeps_the_surrounding_optional_params() {
    let mut optional = map(r#"{"n":2}"#);
    map_params(
        &[
            ("seed".to_owned(), value("3")),
            ("bogus".to_owned(), value("4")),
        ],
        &mut optional,
    );
    assert_eq!(optional, map(r#"{"n":2,"random_seed":3}"#));
}

const PROJECTED: &str = r#"{"id":"cmpl-1","created":1700000000,"model":"codestral-latest","object":"text_completion","usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5},"choices":[{"index":0,"message":{"role":"assistant","content":"<|assistant|>hi"},"finish_reason":"length","logprobs":null},{"index":4,"message":{"content":"second"},"finish_reason":"stop_sequence"}]}"#;

#[test]
fn reply_projects_each_choice_message_content_to_text() {
    let projected = reply(PROJECTED);
    assert_eq!(projected.id, "cmpl-1");
    assert_eq!(projected.object, "text_completion");
    assert_eq!(projected.created, 1_700_000_000);
    assert_eq!(projected.model.as_deref(), Some("codestral-latest"));
    assert_eq!(
        projected.usage,
        CodestralUsage {
            prompt_tokens: 2,
            completion_tokens: 3,
            total_tokens: 5,
        }
    );
    assert_eq!(projected.choices.len(), 2);
    assert_eq!(
        projected.choices[0].text.as_deref(),
        Some("<|assistant|>hi")
    );
    assert_eq!(
        projected.choices[0].finish_reason.as_deref(),
        Some("length")
    );
    assert_eq!(projected.choices[0].logprobs, None);
    assert_eq!(projected.choices[1].index, 4);
    assert_eq!(projected.choices[1].text.as_deref(), Some("second"));
    assert_eq!(projected.choices[1].finish_reason.as_deref(), Some("stop"));
}

#[rstest]
#[case::length("length", Some("length"))]
#[case::stop_sequence("stop_sequence", Some("stop"))]
#[case::empty("", None)]
#[case::unmapped("weird_reason", Some("stop"))]
fn reply_maps_finish_reasons(#[case] raw: &str, #[case] expected: Option<&str>) {
    let body = format!(
        r#"{{"id":"x","created":1,"choices":[{{"index":0,"message":{{"content":"a"}},"finish_reason":"{raw}"}}]}}"#
    );
    assert_eq!(reply(&body).choices[0].finish_reason.as_deref(), expected);
}

#[test]
fn reply_keeps_chatml_markers() {
    let body = r#"{"id":"x","created":1,"choices":[{"index":0,"message":{"content":"<|assistant|><s>text</s>"},"finish_reason":"stop"}]}"#;
    assert_eq!(
        reply(body).choices[0].text.as_deref(),
        Some("<|assistant|><s>text</s>")
    );
}

#[test]
fn reply_defaults_the_missing_top_level_fields() {
    let source = RecordSource {
        id: Uuid::nil(),
        created: 1_700_000_000,
    };
    let raw = r#"{"choices":[]}"#;
    let projected = decode_codestral_reply(200, raw.as_bytes(), &source).expect("a reply");
    assert_eq!(
        projected.id,
        "chatcmpl-00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(projected.created, 1_700_000_000);
    assert_eq!(projected.object, "text_completion");
    assert_eq!(projected.model, None);
    assert_eq!(projected.usage, CodestralUsage::default());
    assert!(projected.choices.is_empty());
}

#[test]
fn reply_keeps_supplied_fields_and_overwrites_the_object() {
    let raw = r#"{"id":"cmpl-9","created":5,"model":"m","object":"chat.completion","usage":{"prompt_tokens":2},"choices":[]}"#;
    let projected = reply(raw);
    assert_eq!(projected.id, "cmpl-9");
    assert_eq!(projected.created, 5);
    assert_eq!(projected.model.as_deref(), Some("m"));
    assert_eq!(projected.object, "text_completion");
    assert_eq!(
        projected.usage,
        CodestralUsage {
            prompt_tokens: 2,
            completion_tokens: 0,
            total_tokens: 0,
        }
    );
}

#[test]
fn reply_keeps_choice_logprobs() {
    let raw = r#"{"id":"x","created":1,"choices":[{"index":0,"message":{"content":"a"},"finish_reason":"stop","logprobs":{"tokens":["a"],"token_logprobs":[null]}}]}"#;
    assert_eq!(
        reply(raw).choices[0].logprobs,
        Some(value(r#"{"tokens":["a"],"token_logprobs":[null]}"#))
    );
}

#[test]
fn reply_requires_exactly_200() {
    let error = decode_codestral_reply(201, b"created", &source()).expect_err("a native error");
    let TextCompletionCodestralError::Native(native) = error else {
        panic!("a native error")
    };
    assert_eq!(native.status_code, 201);
    assert_eq!(native.message, "created");
    assert_eq!(native.request.method, "POST");
    assert_eq!(
        native.request.url,
        "https://docs.codestral.com/user-guide/inference/rest_api"
    );
    assert_eq!(native.response.status_code, 201);
}

#[test]
fn reply_invalid_json_is_a_422_native_error() {
    let error = decode_codestral_reply(200, b"not json", &source()).expect_err("a native error");
    let TextCompletionCodestralError::Native(native) = error else {
        panic!("a native error")
    };
    assert_eq!(native.status_code, 422);
    assert_eq!(native.message, "not json");
    assert_eq!(native.response.status_code, 422);
}

#[test]
fn reply_rejects_json_that_is_not_a_completion_object() {
    let error = decode_codestral_reply(200, b"[1, 2]", &source()).expect_err("an error");
    assert!(
        matches!(error, TextCompletionCodestralError::Native(native) if native.status_code == 422)
    );
}

#[test]
fn reply_treats_a_null_message_as_absent() {
    let raw =
        r#"{"id":"x","created":1,"choices":[{"index":0,"message":null,"finish_reason":"stop"}]}"#;
    assert_eq!(reply(raw).choices[0].text, None);
}

#[test]
fn reply_rejects_a_body_over_the_byte_limit() {
    let body = vec![b' '; MAX_RECORD_BYTES + 1];
    let error = decode_codestral_reply(200, &body, &source()).expect_err("an error");
    assert!(matches!(
        error,
        TextCompletionCodestralError::TooLarge { size, limit }
            if size == MAX_RECORD_BYTES + 1 && limit == MAX_RECORD_BYTES
    ));
}

#[rstest]
#[case::ok_status(200, 422)]
#[case::gateway(502, 502)]
fn reply_decodes_a_body_that_is_not_utf8_lossily(#[case] status: u16, #[case] expected: u16) {
    let error =
        decode_codestral_reply(status, b"\xff\xfe bad gateway", &source()).expect_err("an error");
    let TextCompletionCodestralError::Native(native) = error else {
        panic!("a native error");
    };
    assert_eq!(native.status_code, expected);
    assert_eq!(native.message, "\u{fffd}\u{fffd} bad gateway");
}

#[test]
fn native_error_uses_a_supplied_request_and_response() {
    let request = NativeRequest {
        method: "GET".to_owned(),
        url: "https://example.test/x".to_owned(),
    };
    let response = NativeResponse {
        status_code: 400,
        request: request.clone(),
    };
    let error = NativeError::new(503, "boom".to_owned(), Some(request), Some(response));
    assert_eq!(error.status_code, 503);
    assert_eq!(error.request.method, "GET");
    assert_eq!(error.response.status_code, 400);
}

#[test]
fn post_call_records_the_answer_before_the_status_check() {
    let data = value(r#"{"prompt":"p"}"#);
    let messages = vec![value(r#"{"role":"user","content":"p"}"#)];
    let mut recorder = Recorder { calls: Vec::new() };
    let rejected = PostCall {
        input: &messages,
        api_key: "key",
        original_response: "created",
        request_data: &data,
    };
    let _refused = process_response(201, rejected, &mut recorder, &source());
    assert_eq!(
        recorder.calls,
        vec![("key".to_owned(), "created".to_owned(), data.clone())]
    );
}

#[test]
fn post_call_records_a_successful_answer() {
    let data = value(r#""body""#);
    let body = r#"{"id":"x","created":1,"choices":[]}"#;
    let mut recorder = Recorder { calls: Vec::new() };
    let accepted = PostCall {
        input: &[],
        api_key: "key",
        original_response: body,
        request_data: &data,
    };
    let projected = process_response(200, accepted, &mut recorder, &source()).expect("a reply");
    assert_eq!(projected.id, "x");
    assert_eq!(recorder.calls.len(), 1);
    assert_eq!(recorder.calls[0].1, body);
    assert_eq!(recorder.calls[0].2, data);
}

#[rstest]
#[case::blank("")]
#[case::spaces("   ")]
#[case::done("[DONE]")]
#[case::data_done("data: [DONE]")]
#[case::invalid_json("not json")]
#[case::empty_choices(r#"{"choices": []}"#)]
#[case::null_choices(r#"{"choices": null}"#)]
fn chunk_malformed_lines_are_empty_and_unfinished(#[case] line: &str) {
    let chunk = decode_codestral_chunk(line.as_bytes(), &source()).expect("an empty chunk");
    assert_eq!(chunk, CodestralChunk::default());
}

#[test]
fn chunk_projects_only_the_first_choice_delta() {
    let line = r#"{"id":"c1","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null,"logprobs":{"content":[]}},{"index":1,"delta":{"content":"ignored"},"finish_reason":"stop"}]}"#;
    let chunk = decode_codestral_chunk(line.as_bytes(), &source()).expect("a content chunk");
    let content = chunk;
    assert_eq!(content.text, "hi");
    assert!(!content.is_finished);
    assert_eq!(content.finish_reason, None);
    assert_eq!(content.logprobs, None);
    assert_eq!(
        content
            .original_chunk
            .as_ref()
            .expect("an original chunk")
            .choices
            .len(),
        2
    );
    assert_eq!(
        content
            .original_chunk
            .as_ref()
            .expect("an original chunk")
            .choices[1]
            .delta
            .content
            .as_deref(),
        Some("ignored")
    );
}

#[test]
fn chunk_original_keeps_identity_and_maps_each_finish_reason() {
    let line = r#"{"id":"c1","created":7,"model":"m","object":"chat.completion.chunk","system_fingerprint":"fp","choices":[{"index":2,"delta":{"content":"a","role":"assistant"},"finish_reason":"stop_sequence","logprobs":null}]}"#;
    let chunk = decode_codestral_chunk(line.as_bytes(), &source()).expect("a content chunk");
    let content = chunk;
    let original = content.original_chunk.expect("an original chunk");
    assert_eq!(original.id, "c1");
    assert_eq!(original.created, 7);
    assert_eq!(original.object, "chat.completion.chunk");
    assert_eq!(original.model, "m");
    assert_eq!(original.system_fingerprint.as_deref(), Some("fp"));
    assert_eq!(original.usage, None);
    assert_eq!(original.choices[0].index, 2);
    assert_eq!(original.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(original.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(original.choices[0].delta.function_call, None);
}

#[test]
fn chunk_generates_the_missing_stream_identity() {
    let source = RecordSource {
        id: Uuid::nil(),
        created: 1_700_000_000,
    };
    let line = r#"{"choices":[{"index":0,"delta":{"content":"x"},"finish_reason":null}]}"#;
    let chunk = decode_codestral_chunk(line.as_bytes(), &source).expect("a content chunk");
    let content = chunk;
    assert_eq!(
        content
            .original_chunk
            .as_ref()
            .expect("an original chunk")
            .id,
        "chatcmpl-00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(
        content.original_chunk.expect("an original chunk").created,
        1_700_000_000
    );
}

#[test]
fn chunk_empty_finish_reason_finishes_and_carries_logprobs() {
    let line = r#"{"id":"c2","created":2,"choices":[{"index":0,"delta":{"content":""},"finish_reason":"","logprobs":{"tokens":["a"],"token_logprobs":[null]}}]}"#;
    let content = decode_codestral_chunk(line.as_bytes(), &source()).expect("a content chunk");
    assert!(content.is_finished);
    assert_eq!(content.finish_reason.as_deref(), Some(""));
    assert_eq!(
        serde_json::to_value(&content.logprobs).expect("serializable logprobs"),
        value(r#"{"tokens":["a"],"token_logprobs":[null]}"#)
    );
    let original = content.original_chunk.as_ref().expect("an original chunk");
    assert_eq!(original.choices[0].finish_reason, None);
}

#[test]
fn chunk_without_a_delta_content_is_empty_text() {
    let line = r#"{"id":"c3","created":3,"choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#;
    let chunk = decode_codestral_chunk(line.as_bytes(), &source()).expect("a content chunk");
    let content = chunk;
    assert_eq!(content.text, "");
}

#[rstest]
#[case::sse_space(r#"data: {"choices": [{"delta": {"content": "x"}, "finish_reason": null}]}"#)]
#[case::sse_no_space(r#"data:{"choices": [{"delta": {"content": "x"}, "finish_reason": null}]}"#)]
#[case::padded(r#"  {"choices": [{"delta": {"content": "x"}, "finish_reason": null}]}  "#)]
fn chunk_strips_the_sse_prefix_and_whitespace(#[case] line: &str) {
    let chunk = decode_codestral_chunk(line.as_bytes(), &source()).expect("a content chunk");
    let content = chunk;
    assert_eq!(content.text, "x");
}

#[rstest]
#[case::bad_id(r#"{"id": 5, "choices": []}"#)]
#[case::bad_created(r#"{"created": "x", "choices": []}"#)]
#[case::bad_choice(r#"{"id": "c", "choices": ["bad"]}"#)]
#[case::not_an_object("[1, 2]")]
fn chunk_validates_the_stream_object_before_the_choices(#[case] line: &str) {
    let error = decode_codestral_chunk(line.as_bytes(), &source()).expect_err("an error");
    assert!(matches!(error, TextCompletionCodestralError::Stream(_)));
}

#[test]
fn chunk_rejects_a_body_over_the_byte_limit() {
    let body = vec![b' '; MAX_RECORD_BYTES + 1];
    let error = decode_codestral_chunk(&body, &source()).expect_err("an error");
    assert!(matches!(
        error,
        TextCompletionCodestralError::TooLarge { size, .. } if size == MAX_RECORD_BYTES + 1
    ));
}

#[test]
fn chunk_decodes_a_body_that_is_not_utf8_lossily() {
    let empty = decode_codestral_chunk(&[0xff, 0xfe], &source()).expect("an empty chunk");
    assert_eq!(empty, CodestralChunk::default());
    let line =
        b"{\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a\xff\"},\"finish_reason\":null}]}";
    let chunk = decode_codestral_chunk(line, &source()).expect("a content chunk");
    assert_eq!(chunk.text, "a\u{fffd}");
}

#[rstest]
fn replays_codestral_record_corpus(#[files("fuzz/corpus/codestral_record/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus file");
    let (&tag, body) = bytes.split_first().expect("a tag byte");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a file name");
    let source = RecordSource {
        id: Uuid::nil(),
        created: 0,
    };
    let accepted = match tag {
        b'R' => decode_codestral_reply(200, body, &source).is_ok(),
        b'C' => decode_codestral_chunk(body, &source).is_ok(),
        other => panic!("unknown corpus tag {other}"),
    };
    assert_eq!(accepted, name.starts_with("ok-"), "{name}");
}

#[test]
fn a_body_of_exactly_the_byte_limit_is_decoded() {
    let body = vec![b' '; MAX_RECORD_BYTES];
    let chunk = decode_codestral_chunk(&body, &source()).expect("a decoded chunk");
    assert_eq!(chunk, CodestralChunk::default());
    let error = decode_codestral_reply(200, &body, &source()).expect_err("an error");
    assert!(matches!(error, TextCompletionCodestralError::Native(_)));
}
