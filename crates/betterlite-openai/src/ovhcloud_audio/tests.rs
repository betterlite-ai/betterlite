#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::reply::decode_transcription_reply;
use super::request::{
    complete_url, map_openai_params, transcription_request, validate_environment,
};
use crate::api::audio_file_input::ProcessedAudioFile;
use crate::openai_base_config::{Headers, Object};

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

#[rstest]
#[case::default_base(
    None,
    "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/audio/transcriptions"
)]
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
#[case::no_key_at_all(None, None, "Bearer None")]
#[case::explicit_key(Some("explicit"), None, "Bearer explicit")]
#[case::env_key(None, Some("env"), "Bearer env")]
#[case::explicit_key_wins(Some("explicit"), Some("env"), "Bearer explicit")]
fn validate_environment_writes_the_bearer_default(
    #[case] api_key: Option<&str>,
    #[case] env_api_key: Option<&str>,
    #[case] authorization: &str,
) {
    let explicit = api_key.map(SecretString::from);
    let from_env = env_api_key.map(SecretString::from);
    let headers = validate_environment(&Headers::new(), explicit.as_ref(), from_env.as_ref());
    assert_eq!(
        headers,
        Headers::from([
            (String::from("Authorization"), String::from(authorization)),
            (String::from("accept"), String::from("application/json")),
        ])
    );
}

// the audio config never sends a Content-Type, and a caller entry of its own rides along untouched
#[test]
fn validate_environment_lays_the_caller_headers_over_the_defaults() {
    let caller = Headers::from([
        (String::from("accept"), String::from("text/plain")),
        (String::from("x-extra"), String::from("1")),
    ]);
    let headers = validate_environment(&caller, None, None);
    assert_eq!(
        headers,
        Headers::from([
            (String::from("Authorization"), String::from("Bearer None")),
            (String::from("accept"), String::from("text/plain")),
            (String::from("x-extra"), String::from("1")),
        ])
    );
}

#[test]
fn map_openai_params_copies_only_the_supported_params() {
    let non_default = object(json!({"prompt": "p", "temperature": 0, "top_p": 0.5, "model": "m"}));
    let mut optional = object(json!({"response_format": "json", "top_p": 0.9}));
    map_openai_params(&non_default, &mut optional);
    assert_eq!(
        serde_json::to_value(&optional).expect("options"),
        json!({"response_format": "json", "prompt": "p", "temperature": 0, "top_p": 0.9})
    );
}

#[test]
fn transcription_request_keeps_model_first_then_the_supported_params_in_order() {
    let optional = object(json!({
        "temperature": 0.2,
        "timestamp_granularities": ["word"],
        "response_format": "verbose_json",
        "top_p": 0.5,
        "prompt": null,
        "language": "fr",
    }));
    let request = transcription_request("whisper-large-v3", &audio(), &optional);
    assert_eq!(
        request.fields,
        vec![
            (String::from("model"), json!("whisper-large-v3")),
            (String::from("language"), json!("fr")),
            (String::from("response_format"), json!("verbose_json")),
            (String::from("timestamp_granularities"), json!(["word"])),
            (String::from("temperature"), json!(0.2)),
        ]
    );
    assert_eq!(request.file, audio());
}

#[test]
fn transcription_request_skips_every_unset_supported_param() {
    let request = transcription_request("m", &audio(), &object(json!({})));
    assert_eq!(
        request.fields,
        vec![(String::from("model"), Value::String(String::from("m")))]
    );
}

#[rstest]
#[case::text_is_kept(json!({"text": "hello"}), "hello", json!({"text": "hello"}))]
#[case::transcript_is_the_fallback(json!({"transcript": "hello"}), "hello", json!({"transcript": "hello"}))]
#[case::text_wins_over_transcript(
    json!({"text": "one", "transcript": "two"}),
    "one",
    json!({"text": "one", "transcript": "two"})
)]
#[case::an_empty_text_falls_through_to_transcript(
    json!({"text": "", "transcript": "two"}),
    "two",
    json!({"text": "", "transcript": "two"})
)]
#[case::an_empty_body_gives_the_empty_string(json!({}), "", json!({}))]
fn decode_transcription_reply_reads_the_text(
    #[case] source: serde_json::Value,
    #[case] text: &str,
    #[case] hidden: serde_json::Value,
) {
    let body = serde_json::to_vec(&source).expect("bytes");
    let transcription =
        decode_transcription_reply(&body, 200, &Headers::new()).expect("a transcription");
    assert_eq!(transcription.response.text.as_deref(), Some(text));
    assert_eq!(
        serde_json::to_value(&transcription.hidden).expect("hidden"),
        hidden
    );
}

#[rstest]
#[case::seconds_wins(json!({"seconds": 3, "duration": 9}), json!({"seconds": 3, "duration": 3}))]
#[case::a_zero_second_is_kept(json!({"seconds": 0}), json!({"seconds": 0, "duration": 0}))]
#[case::a_null_second_falls_back(json!({"seconds": null, "duration": 9}), json!({"seconds": null, "duration": 9}))]
#[case::duration_alone_is_written_back(json!({"duration": 9}), json!({"duration": 9}))]
#[case::a_null_duration_stays_null(json!({"duration": null}), json!({"duration": null}))]
#[case::no_duration_key_adds_none(json!({"text": "x"}), json!({"text": "x"}))]
fn decode_transcription_reply_normalizes_the_duration(
    #[case] source: serde_json::Value,
    #[case] hidden: serde_json::Value,
) {
    let body = serde_json::to_vec(&source).expect("bytes");
    let transcription =
        decode_transcription_reply(&body, 200, &Headers::new()).expect("a transcription");
    assert_eq!(
        serde_json::to_value(&transcription.hidden).expect("hidden"),
        hidden
    );
}

#[test]
fn decode_transcription_reply_reports_a_body_that_is_not_json() {
    let headers = Headers::from([(String::from("x-request-id"), String::from("req-1"))]);
    let error = decode_transcription_reply(b"oops", 503, &headers).expect_err("an error");
    assert_eq!(error.message, "oops");
    assert_eq!(error.status_code, Some(503));
    assert_eq!(error.headers, headers);
}

#[test]
fn decode_transcription_reply_reports_a_json_body_that_is_not_an_object() {
    let error = decode_transcription_reply(b"[]", 200, &Headers::new()).expect_err("an error");
    assert_eq!(error.status_code, Some(200));
}
