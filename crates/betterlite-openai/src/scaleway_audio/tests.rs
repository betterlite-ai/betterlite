#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::error::MISSING_KEY_MESSAGE;
use super::reply::decode_transcription_reply;
use super::request::{
    complete_url, map_openai_params, transcription_request, validate_environment,
};
use crate::api::audio_file_input::ProcessedAudioFile;
use crate::openai_base_config::{Headers, Object};
use crate::ovhcloud_chat::error::get_error_class;

fn object(value: serde_json::Value) -> Object {
    serde_json::from_value(value).expect("a JSON object")
}

fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"RIFF".to_vec(),
        filename: String::from("clip.wav"),
        content_type: String::from("audio/wav"),
    }
}

fn json_headers() -> Headers {
    Headers::from([(
        String::from("content-type"),
        String::from("application/json"),
    )])
}

#[rstest]
#[case::default_base(None, "https://api.scaleway.ai/v1/audio/transcriptions")]
#[case::given_base(
    Some("https://proxy.example/v1"),
    "https://proxy.example/v1/audio/transcriptions"
)]
#[case::trailing_slashes_are_stripped(
    Some("https://proxy.example/v1//"),
    "https://proxy.example/v1/audio/transcriptions"
)]
#[case::empty_base_stays_empty(Some(""), "/audio/transcriptions")]
fn complete_url_uses_the_default_base_or_the_given_one(
    #[case] api_base: Option<&str>,
    #[case] url: &str,
) {
    assert_eq!(complete_url(api_base), url);
}

#[rstest]
#[case::no_key_at_all(None, None)]
#[case::an_empty_explicit_key(Some(""), None)]
#[case::an_empty_env_key(None, Some(""))]
fn validate_environment_rejects_a_missing_key(
    #[case] api_key: Option<&str>,
    #[case] env_api_key: Option<&str>,
) {
    let explicit = api_key.map(SecretString::from);
    let from_env = env_api_key.map(SecretString::from);
    let error = validate_environment(&Headers::new(), explicit.as_ref(), from_env.as_ref())
        .expect_err("a missing key");
    assert_eq!(error.status_code, 401);
    assert_eq!(error.message, MISSING_KEY_MESSAGE);
    assert_eq!(
        error.message,
        "Scaleway API key not found. Pass `api_key=...` or set the SCW_SECRET_KEY environment variable."
    );
    assert!(error.headers.is_empty());
}

#[rstest]
#[case::explicit_key(Some("explicit"), Some("env"), "Bearer explicit")]
#[case::env_key(None, Some("env"), "Bearer env")]
fn validate_environment_builds_the_default_headers(
    #[case] api_key: Option<&str>,
    #[case] env_api_key: Option<&str>,
    #[case] authorization: &str,
) {
    let explicit = api_key.map(SecretString::from);
    let from_env = env_api_key.map(SecretString::from);
    let headers = validate_environment(&Headers::new(), explicit.as_ref(), from_env.as_ref())
        .expect("headers");
    assert_eq!(
        headers,
        Headers::from([
            (String::from("Authorization"), String::from(authorization)),
            (String::from("accept"), String::from("application/json")),
        ])
    );
}

#[test]
fn validate_environment_lays_the_caller_headers_over_the_defaults() {
    let caller = Headers::from([(String::from("Authorization"), String::from("Bearer caller"))]);
    let headers = validate_environment(&caller, Some(&SecretString::from("explicit")), None)
        .expect("headers");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn get_error_class_keeps_the_message_status_and_headers() {
    let headers = json_headers();
    let error = get_error_class("bad request", 422, &headers);
    assert_eq!(error.status_code, 422);
    assert_eq!(error.message, "bad request");
    assert_eq!(error.headers, headers);
}

#[test]
fn map_openai_params_keeps_the_given_order_and_drops_the_rest() {
    let non_default =
        object(json!({"timestamp_granularities": ["word"], "top_k": 3, "language": "en"}));
    let mut optional = object(json!({}));
    map_openai_params(&non_default, &mut optional);
    assert_eq!(
        serde_json::to_value(&optional).expect("options"),
        json!({"language": "en", "timestamp_granularities": ["word"]})
    );
}

#[test]
fn transcription_request_puts_temperature_before_timestamp_granularities() {
    let optional = object(json!({
        "timestamp_granularities": ["word"],
        "temperature": 0.1,
        "response_format": "json",
        "prompt": null,
    }));
    let request = transcription_request("whisper-large-v3", &audio(), &optional);
    assert_eq!(
        request.fields,
        vec![
            (String::from("model"), json!("whisper-large-v3")),
            (String::from("response_format"), json!("json")),
            (String::from("temperature"), json!(0.1)),
            (String::from("timestamp_granularities"), json!(["word"])),
        ]
    );
    assert_eq!(request.file, audio());
}

#[rstest]
#[case::no_content_type(&[], "raw text")]
#[case::a_text_content_type(&[("content-type", "text/plain")], "raw text")]
#[case::an_upper_case_content_type(&[("Content-Type", "TEXT/PLAIN")], "raw text")]
fn a_reply_that_is_not_labelled_json_comes_back_as_text(
    #[case] header_pairs: &[(&str, &str)],
    #[case] text: &str,
) {
    let headers = Headers::from_iter(
        header_pairs
            .iter()
            .map(|(name, value)| (String::from(*name), String::from(*value))),
    );
    let transcription =
        decode_transcription_reply(b"raw text", 200, &headers).expect("a transcription");
    assert_eq!(transcription.response.text.as_deref(), Some(text));
    assert!(transcription.hidden.is_empty());
    assert!(transcription.response.extra.is_empty());
}

#[rstest]
#[case::the_text_and_the_two_fields(
    json!({"text": "hello", "segments": [{"id": 0}], "language": "fr", "duration": 2}),
    "hello",
    json!({"segments": [{"id": 0}], "language": "fr"})
)]
#[case::an_absent_text_is_empty(json!({"language": "fr"}), "", json!({"language": "fr"}))]
#[case::a_null_text_is_empty(json!({"text": null, "segments": []}), "", json!({"segments": []}))]
fn a_json_reply_copies_the_text_segments_and_language(
    #[case] source: serde_json::Value,
    #[case] text: &str,
    #[case] extra: serde_json::Value,
) {
    let body = serde_json::to_vec(&source).expect("bytes");
    let transcription =
        decode_transcription_reply(&body, 200, &json_headers()).expect("a transcription");
    assert_eq!(transcription.response.text.as_deref(), Some(text));
    assert_eq!(
        serde_json::to_value(&transcription.response.extra).expect("extra"),
        extra
    );
    assert_eq!(
        serde_json::to_value(&transcription.hidden).expect("hidden"),
        source
    );
}

#[test]
fn a_json_reply_that_does_not_parse_carries_the_body_text_and_the_status() {
    let headers = Headers::from([
        (
            String::from("Content-Type"),
            String::from("application/json; charset=utf-8"),
        ),
        (String::from("x-request-id"), String::from("req-1")),
    ]);
    let error = decode_transcription_reply(b"oops", 502, &headers).expect_err("an error");
    assert_eq!(error.message, "oops");
    assert_eq!(error.status_code, 502);
    assert_eq!(error.headers, headers);
}

#[test]
fn a_json_reply_that_is_not_an_object_carries_the_status() {
    let error = decode_transcription_reply(b"[]", 200, &json_headers()).expect_err("an error");
    assert_eq!(error.status_code, 200);
}

#[test]
fn a_reply_over_the_size_limit_is_rejected_on_the_json_branch() {
    let body = vec![b' '; super::MAX_REPLY_BYTES + 1];
    let error = decode_transcription_reply(&body, 200, &json_headers()).expect_err("an error");
    assert_eq!(error.status_code, 200);
}
