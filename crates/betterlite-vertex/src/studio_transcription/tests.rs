#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::Object;
use http::header::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::studio_interactions::API_REVISION;
use crate::studio_interactions::credentials::StudioParams;
use crate::studio_transcription::decode::decode_studio_transcription;
use crate::studio_transcription::error::StudioTranscriptionError;
use crate::studio_transcription::request::{
    map_openai_params, transform_audio_request, validate_environment,
};
use crate::studio_transcription::{SUPPORTED_PARAMS, SUPPORTS_SUBTITLE_SYNTHESIS};

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

fn settings(key: Option<&str>) -> Settings {
    Settings {
        environment_key: key.map(secret),
        generic_api_key: None,
        api_base: "https://generativelanguage.googleapis.com".to_owned(),
    }
}

fn params(key: Option<&str>) -> StudioParams {
    StudioParams {
        api_key: key.map(secret),
    }
}

fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"audio".to_vec(),
        filename: "clip.flac".to_owned(),
        content_type: "audio/flac".to_owned(),
    }
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn reply(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn decode_bytes(
    status: u16,
    body: &[u8],
) -> Result<TranscriptionResponse, StudioTranscriptionError> {
    decode_studio_transcription(&UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    })
}

fn answer(status: u16, body: &str) -> Object {
    let decoded = decode_studio_transcription(&reply(status, body)).unwrap();
    serde_json::from_value(serde_json::to_value(decoded).unwrap()).unwrap()
}

fn answer_bytes(status: u16, body: &[u8]) -> Object {
    let decoded = decode_bytes(status, body).unwrap();
    serde_json::from_value(serde_json::to_value(decoded).unwrap()).unwrap()
}

fn request_body(model: &str, optional: &str) -> Object {
    let request = transform_audio_request(model, &audio(), &object(optional)).unwrap();
    serde_json::from_value(serde_json::to_value(request.data).unwrap()).unwrap()
}

fn bare_request(model: &str) -> Object {
    object(&format!(
        r#"{{"model":"{model}","input":[{{"type":"audio","data":"YXVkaW8=","mime_type":"audio/flac"}}]}}"#
    ))
}

fn configured_request(model: &str, config: &str) -> Object {
    let mut request = bare_request(model);
    request.insert(
        "generation_config".to_owned(),
        serde_json::from_str(config).unwrap(),
    );
    request
}

fn verbatim_mode() -> Object {
    configured_request(
        "gemini-3.5-transcribe",
        r#"{"transcription_config":{"mode":{"type":"verbatim","timestamp_granularities":["word"],"diarization_mode":"speaker"}}}"#,
    )
}

#[test]
fn the_advertised_params_are_the_three_transcription_options() {
    assert_eq!(
        SUPPORTED_PARAMS,
        ["language", "response_format", "timestamp_granularities"]
    );
}

#[test]
fn the_config_advertises_subtitle_synthesis() {
    const { assert!(SUPPORTS_SUBTITLE_SYNTHESIS) };
}

#[test]
fn a_bare_request_omits_generation_config() {
    assert_eq!(
        request_body("gemini/gemini-3.5-transcribe", "{}"),
        bare_request("gemini-3.5-transcribe")
    );
}

#[rstest]
#[case::word(&["word"], true)]
#[case::word_among_segments(&["segment", "word"], true)]
#[case::segments_only(&["segment"], false)]
#[case::empty(&[], false)]
fn the_word_granularity_enables_verbatim_mode(
    #[case] granularities: &[&str],
    #[case] configured: bool,
) {
    let listed = granularities
        .iter()
        .map(|granularity| format!(r#""{granularity}""#))
        .collect::<Vec<_>>()
        .join(",");
    let optional = format!(r#"{{"timestamp_granularities":[{listed}]}}"#);
    let body = request_body("gemini/gemini-3.5-transcribe", &optional);
    let expected = if configured {
        verbatim_mode()
    } else {
        bare_request("gemini-3.5-transcribe")
    };
    assert_eq!(body, expected);
}

#[rstest]
#[case::srt(r#"{"response_format":"srt"}"#, true)]
#[case::vtt(r#"{"response_format":"vtt"}"#, true)]
#[case::json(r#"{"response_format":"json"}"#, false)]
#[case::text(r#"{"response_format":"text"}"#, false)]
#[case::verbose_json(r#"{"response_format":"verbose_json"}"#, false)]
fn a_subtitle_response_format_enables_diarization(#[case] optional: &str, #[case] diarized: bool) {
    let body = request_body("gemini/gemini-3.5-transcribe", optional);
    assert_eq!(body == verbatim_mode(), diarized);
    assert_eq!(body == bare_request("gemini-3.5-transcribe"), !diarized);
}

#[test]
fn language_and_word_mode_configure_independently() {
    assert_eq!(
        request_body("gemini/gemini-3.5-transcribe", r#"{"language":"en"}"#),
        configured_request(
            "gemini-3.5-transcribe",
            r#"{"transcription_config":{"language_codes":["en-US"]}}"#
        )
    );
    assert_eq!(
        request_body(
            "gemini/gemini-3.5-transcribe",
            r#"{"language":"pt-BR","timestamp_granularities":["word"]}"#
        ),
        configured_request(
            "gemini-3.5-transcribe",
            r#"{"transcription_config":{"language_codes":["pt-BR"],"mode":{"type":"verbatim","timestamp_granularities":["word"],"diarization_mode":"speaker"}}}"#
        )
    );
}

#[rstest]
#[case::bare_code(r#"{"language":"fr"}"#, "fr-FR")]
#[case::region_kept(r#"{"language":"zh-Hant"}"#, "zh-Hant")]
#[case::unknown_kept(r#"{"language":"xx"}"#, "xx")]
#[case::empty_ignored(r#"{"language":""}"#, "")]
#[case::null_ignored(r#"{"language":null}"#, "")]
#[case::number_ignored(r#"{"language":7}"#, "")]
fn the_language_is_normalized_to_bcp47(#[case] optional: &str, #[case] expected: &str) {
    let body = request_body("gemini/gemini-3.5-transcribe", optional);
    let wanted = if expected.is_empty() {
        bare_request("gemini-3.5-transcribe")
    } else {
        configured_request(
            "gemini-3.5-transcribe",
            &format!(r#"{{"transcription_config":{{"language_codes":["{expected}"]}}}}"#),
        )
    };
    assert_eq!(body, wanted);
}

#[rstest]
#[case::one_prefix("gemini/gemini-3.5-transcribe", "gemini-3.5-transcribe")]
#[case::nested_prefix("gemini/gemini/gemini-3.5-transcribe", "gemini/gemini-3.5-transcribe")]
#[case::no_prefix("gemini-3.5-transcribe", "gemini-3.5-transcribe")]
fn the_model_loses_one_gemini_prefix(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(request_body(model, "{}"), bare_request(expected));
}

#[test]
fn supported_params_overlay_the_caller_options() {
    let optional = object(r#"{"response_format":"json","language":"fr","temperature":1}"#);
    let non_default =
        object(r#"{"response_format":"vtt","timestamp_granularities":["word"],"prompt":"x"}"#);
    assert_eq!(
        map_openai_params(&non_default, &optional),
        object(
            r#"{"language":"fr","response_format":"vtt","temperature":1,"timestamp_granularities":["word"]}"#
        )
    );
}

#[rstest]
#[case::no_key(None, None)]
#[case::empty_key(Some(""), None)]
fn a_missing_api_key_is_unauthorized(#[case] param: Option<&str>, #[case] env: Option<&str>) {
    let error = validate_environment(&IndexMap::new(), &params(param), &settings(env)).unwrap_err();
    assert!(matches!(error, StudioTranscriptionError::MissingApiKey));
    assert_eq!(error.status_code(), Some(401));
}

#[rstest]
#[case::param_key(Some("param-key"), None, "param-key")]
#[case::environment_key(None, Some("env-key"), "env-key")]
#[case::empty_param_uses_environment(Some(""), Some("env-key"), "env-key")]
fn the_key_comes_from_params_then_the_environment(
    #[case] param: Option<&str>,
    #[case] env: Option<&str>,
    #[case] expected: &str,
) {
    let headers = validate_environment(&IndexMap::new(), &params(param), &settings(env)).unwrap();
    assert_eq!(
        headers
            .get("x-goog-api-key")
            .map(secrecy::ExposeSecret::expose_secret),
        Some(expected)
    );
}

#[test]
fn the_json_key_and_revision_headers_override_the_caller() {
    let caller: IndexMap<String, String> = [
        ("Content-Type", "text/plain"),
        ("x-goog-api-key", "caller-key"),
        ("Api-Revision", "2020-01-01"),
        ("X-Caller", "kept"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();
    let headers =
        validate_environment(&caller, &params(Some("real-key")), &settings(None)).unwrap();
    let plain: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    assert_eq!(
        plain,
        [
            ("Content-Type", "application/json"),
            ("x-goog-api-key", "real-key"),
            ("Api-Revision", API_REVISION),
            ("X-Caller", "kept"),
        ]
    );
}

#[test]
fn every_text_content_joins_with_one_space() {
    let body = r#"{"status":"completed","steps":[
        {"content":[{"type":"text","text":"one"},{"type":"image","text":"ignored"},{"type":"text","text":null}]},
        {"content":[{"type":"text","text":""},{"type":"text","text":"two"}]}
    ]}"#;
    let value = answer(200, body);
    assert_eq!(value["text"], "one  two");
    assert_eq!(value["task"], "transcribe");
}

#[test]
fn only_word_info_annotations_with_text_become_words() {
    let body = r#"{"status":"completed","steps":[{"content":[{"type":"text","text":"a","annotations":[
        {"type":"word_info","text":"kept","start_offset":"1s","speaker":""},
        {"type":"speaker_turn","text":"skipped","start_offset":"2s"},
        {"type":"word_info","text":null,"start_offset":"3s"},
        {"text":"missing type"}
    ]}]}]}"#;
    let value = answer(200, body);
    assert_eq!(
        value["words"].to_string(),
        r#"[{"speaker":"","start":1.0,"word":"kept"}]"#
    );
    assert!(value.get("duration").is_none());
}

#[rstest]
#[case::seconds(Some("2.5s"), Some("2.5"))]
#[case::integral(Some("2s"), Some("2.0"))]
#[case::scientific(Some("1e3s"), Some("1000.0"))]
#[case::negative(Some("-1.5s"), Some("-1.5"))]
#[case::millis(Some("2ms"), None)]
#[case::suffix_only(Some("s"), None)]
#[case::text(Some("abc"), None)]
#[case::absent(None, None)]
fn an_offset_renders_seconds_or_disappears(
    #[case] offset: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let encoded = serde_json::to_string(&offset).unwrap();
    let body = format!(
        r#"{{"status":"completed","steps":[{{"content":[{{"type":"text","text":"w","annotations":[{{"type":"word_info","text":"w","start_offset":{encoded}}}]}}]}}]}}"#
    );
    let value = answer(200, &body);
    let words = value["words"].as_array().unwrap();
    let start = words.first().unwrap().get("start").map(ToString::to_string);
    assert_eq!(start.as_deref(), expected);
}

#[test]
fn the_duration_is_the_last_words_end() {
    let body = r#"{"status":"completed","steps":[{"content":[{"type":"text","text":"a b","annotations":[
        {"type":"word_info","text":"a","end_offset":"5s"},
        {"type":"word_info","text":"b","end_offset":"2s"}
    ]}]}]}"#;
    let value = answer(200, body);
    assert_eq!(value["duration"].to_string(), "2.0");
    assert_eq!(value["words"].as_array().unwrap().len(), 2);
}

#[test]
fn a_last_word_without_end_omits_the_duration() {
    let body = r#"{"status":"completed","steps":[{"content":[{"type":"text","text":"a b","annotations":[
        {"type":"word_info","text":"a","end_offset":"5s"},
        {"type":"word_info","text":"b","start_offset":"0.6s"}
    ]}]}]}"#;
    let value = answer(200, body);
    assert!(value.get("duration").is_none());
    assert_eq!(value["words"].as_array().unwrap().len(), 2);
}

#[test]
fn usage_counts_only_the_exact_lowercase_audio_modality() {
    let body = r#"{"status":"completed","usage":{"total_tokens":10,"total_input_tokens":2,"total_output_tokens":8,"input_tokens_by_modality":[
        {"modality":"audio","tokens":5},
        {"modality":"Audio","tokens":3},
        {"modality":"text","tokens":1}
    ]}}"#;
    let value = answer(200, body);
    assert_eq!(
        value["usage"].to_string(),
        r#"{"input_token_details":{"audio_tokens":5,"text_tokens":-3},"input_tokens":2,"output_tokens":8,"total_tokens":10,"type":"tokens"}"#
    );
}

#[test]
fn an_absent_usage_stays_absent_and_an_empty_one_counts_zero() {
    let absent = answer(200, r#"{"status":"completed","steps":[]}"#);
    assert!(absent.get("usage").is_none());
    let empty = answer(
        200,
        r#"{"status":"completed","usage":{"total_tokens":3,"input_tokens_by_modality":[]}}"#,
    );
    assert_eq!(
        empty["usage"].to_string(),
        r#"{"input_token_details":{"audio_tokens":0,"text_tokens":0},"input_tokens":0,"output_tokens":0,"total_tokens":3,"type":"tokens"}"#
    );
}

#[test]
fn overflowing_token_sums_are_refused() {
    let sum = format!(
        r#"{{"status":"completed","usage":{{"input_tokens_by_modality":[{{"modality":"audio","tokens":{}}},{{"modality":"audio","tokens":1}}]}}}}"#,
        i64::MAX
    );
    let sum_error = decode_studio_transcription(&reply(200, &sum)).unwrap_err();
    assert!(matches!(sum_error, StudioTranscriptionError::UsageOverflow));
    assert_eq!(sum_error.status_code(), None);
    let difference = format!(
        r#"{{"status":"completed","usage":{{"total_input_tokens":{},"input_tokens_by_modality":[{{"modality":"audio","tokens":1}}]}}}}"#,
        i64::MIN
    );
    let difference_error = decode_studio_transcription(&reply(200, &difference)).unwrap_err();
    assert!(matches!(
        difference_error,
        StudioTranscriptionError::UsageOverflow
    ));
}

#[rstest]
#[case::in_progress(200, r#"{"status":"in_progress","steps":[]}"#)]
#[case::missing_status(500, r#"{"steps":[]}"#)]
#[case::empty_object(201, "{}")]
#[case::capitalized(200, r#"{"status":"Completed"}"#)]
#[case::empty_status(200, r#"{"status":""}"#)]
fn an_unfinished_interaction_keeps_the_reply_status(#[case] status: u16, #[case] body: &str) {
    let error = decode_studio_transcription(&reply(status, body)).unwrap_err();
    assert!(matches!(error, StudioTranscriptionError::Incomplete { .. }));
    assert_eq!(error.status_code(), Some(status));
}

#[rstest]
#[case::text(502, "not json")]
#[case::empty(500, "")]
#[case::trailing(200, r#"{"status":"completed"} tail"#)]
#[case::binary(503, "\u{1}")]
fn a_non_json_reply_keeps_the_reply_status(#[case] status: u16, #[case] body: &str) {
    let error = decode_studio_transcription(&reply(status, body)).unwrap_err();
    assert!(matches!(error, StudioTranscriptionError::NotJson { .. }));
    assert_eq!(error.status_code(), Some(status));
}

#[rstest]
#[case::root_array(r#"[{"status":"completed"}]"#)]
#[case::null_steps(r#"{"status":"completed","steps":null}"#)]
#[case::annotations_text(
    r#"{"status":"completed","steps":[{"content":[{"annotations":"none"}]}]}"#
)]
#[case::offset_number(
    r#"{"status":"completed","steps":[{"content":[{"annotations":[{"start_offset":5}]}]}]}"#
)]
#[case::tokens_text(r#"{"status":"completed","usage":{"total_tokens":"many"}}"#)]
#[case::status_number(r#"{"status":7}"#)]
fn a_malformed_record_is_a_shape_error(#[case] body: &str) {
    let error = decode_studio_transcription(&reply(200, body)).unwrap_err();
    assert!(matches!(error, StudioTranscriptionError::Shape { .. }));
    assert_eq!(error.status_code(), None);
}

#[test]
fn a_reply_over_the_size_limit_is_refused() {
    let prefix = r#"{"status":"completed"}"#;
    let at_limit = format!("{prefix}{}", " ".repeat(MAX_RESPONSE_BYTES - prefix.len()));
    assert_eq!(at_limit.len(), MAX_RESPONSE_BYTES);
    assert_eq!(answer(200, &at_limit)["text"], "");
    let error = decode_studio_transcription(&reply(200, &format!("{at_limit} "))).unwrap_err();
    assert!(matches!(
        error,
        StudioTranscriptionError::TooLarge { size } if size == MAX_RESPONSE_BYTES + 1
    ));
    assert_eq!(error.status_code(), None);
}

#[rstest]
fn replays_the_studio_transcription_corpus(
    #[files("fuzz/corpus/studio_transcription/*")] path: PathBuf,
) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let body = fs::read(&path).unwrap();
    match name {
        "valid-transcript.json" => {
            let value = answer_bytes(200, &body);
            assert_eq!(value["text"], "hello");
            assert_eq!(
                value["words"].to_string(),
                r#"[{"end":0.5,"speaker":"1","start":0.0,"word":"hello"}]"#
            );
            assert_eq!(value["duration"].to_string(), "0.5");
            assert_eq!(
                value["usage"].to_string(),
                r#"{"input_token_details":{"audio_tokens":4,"text_tokens":1},"input_tokens":5,"output_tokens":2,"total_tokens":7,"type":"tokens"}"#
            );
        }
        "valid-empty-steps.json" => {
            let value = answer_bytes(200, &body);
            assert_eq!(value["text"], "");
            assert_eq!(value["task"], "transcribe");
            assert!(value.get("words").is_none());
        }
        "valid-empty-text.json" => {
            assert_eq!(answer_bytes(200, &body)["text"], "a  b");
        }
        "valid-annotation-types.json" => {
            let value = answer_bytes(200, &body);
            assert_eq!(value["text"], " kept");
            assert_eq!(
                value["words"].to_string(),
                r#"[{"end":1.5,"word":"empty"},{"end":3.25,"word":"last"}]"#
            );
            assert_eq!(value["duration"].to_string(), "3.25");
        }
        "valid-invalid-offsets.json" => {
            let value = answer_bytes(200, &body);
            assert_eq!(
                value["words"].to_string(),
                r#"[{"word":"x"},{"start":1000.0,"word":"y"}]"#
            );
            assert!(value.get("duration").is_none());
        }
        "valid-last-word-end.json" => {
            assert_eq!(answer_bytes(200, &body)["duration"].to_string(), "2.0");
        }
        "valid-last-word-missing-end.json" => {
            let value = answer_bytes(200, &body);
            assert!(value.get("duration").is_none());
            assert_eq!(value["words"].as_array().unwrap().len(), 2);
        }
        "valid-negative-text-tokens.json" => {
            let value = answer_bytes(200, &body);
            assert_eq!(
                value["usage"]["input_token_details"].to_string(),
                r#"{"audio_tokens":5,"text_tokens":-3}"#
            );
        }
        "valid-extra-fields.json" => {
            let value = answer_bytes(200, &body);
            assert_eq!(value["text"], "");
            assert!(value.get("usage").is_none());
        }
        "invalid-status-incomplete.json" | "invalid-status-missing.json" => {
            let error = decode_bytes(200, &body).unwrap_err();
            assert!(matches!(error, StudioTranscriptionError::Incomplete { .. }));
            assert_eq!(error.status_code(), Some(200));
        }
        "invalid-annotations-shape.json" | "malformed-root-array.json" => {
            let error = decode_bytes(200, &body).unwrap_err();
            assert!(matches!(error, StudioTranscriptionError::Shape { .. }));
        }
        "malformed-not-json.json" => {
            let error = decode_bytes(503, &body).unwrap_err();
            assert!(matches!(error, StudioTranscriptionError::NotJson { .. }));
            assert_eq!(error.status_code(), Some(503));
        }
        other => panic!("unexpected seed {other}"),
    }
}
