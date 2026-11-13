use betterlite_config::sections::mistral_resources::Settings;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::super::super::error::MistralAudioError;
use super::super::super::reply::REPLY_LIMIT;
use super::super::super::transcription::{
    DEFAULT_BASE, OPENAI_PARAMS, complete_url, decode_voxtral_transcript, map_openai_params,
    transcription_request, validate_environment,
};
use super::super::{object, response};
use crate::api::audio_file_input::ProcessedAudioFile;
use crate::api::transcription_contract::TranscriptionResponse;

fn settings(api_key: Option<&str>) -> Settings {
    Settings {
        api_base: None,
        api_key: api_key.map(SecretString::from),
    }
}

fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"abc".to_vec(),
        filename: String::from("clip.wav"),
        content_type: String::from("audio/wav"),
    }
}

#[test]
fn declares_the_four_openai_params() {
    assert_eq!(
        OPENAI_PARAMS,
        [
            "language",
            "temperature",
            "timestamp_granularities",
            "response_format"
        ]
    );
}

#[test]
fn copies_only_the_supported_params_into_the_request_options() {
    let non_default =
        object(r#"{"language":"en","diarize":true,"model":"x","response_format":"json"}"#);
    let mut optional = object(r#"{"temperature":0.5}"#);
    map_openai_params(&non_default, &mut optional);
    assert_eq!(
        optional,
        object(r#"{"temperature":0.5,"language":"en","response_format":"json"}"#)
    );
}

#[rstest]
#[case::explicit(Some("explicit"), None, "Bearer explicit")]
#[case::environment(None, Some("environment"), "Bearer environment")]
#[case::explicit_wins(Some("explicit"), Some("environment"), "Bearer explicit")]
#[case::missing(None, None, "Bearer None")]
fn serializes_the_authorization(
    #[case] api_key: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let key = api_key.map(SecretString::from);
    let resolved = validate_environment(&headers(&[]), key.as_ref(), &settings(environment));
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some(expected)
    );
    assert_eq!(
        resolved.get("accept").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn caller_headers_win_only_on_an_exact_name() {
    let given = headers(&[
        ("Authorization", "Bearer mine"),
        ("authorization", "Bearer old"),
    ]);
    let key = SecretString::from("key");
    let resolved = validate_environment(&given, Some(&key), &settings(None));
    assert_eq!(
        resolved,
        headers(&[
            ("Authorization", "Bearer key"),
            ("accept", "application/json"),
            ("authorization", "Bearer old"),
        ])
    );
}

#[rstest]
#[case::default(None, "https://api.mistral.ai/v1/audio/transcriptions")]
#[case::empty(Some(""), "/audio/transcriptions")]
#[case::unversioned(Some("https://mock"), "https://mock/audio/transcriptions")]
#[case::trailing_slash(Some("https://mock/"), "https://mock/audio/transcriptions")]
#[case::versioned(Some("https://mock/v1"), "https://mock/v1/audio/transcriptions")]
#[case::doubled_version(Some("https://mock/v1/v1/"), "https://mock/v1/v1/audio/transcriptions")]
fn builds_the_transcription_url(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(complete_url(api_base), expected);
}

#[test]
fn the_default_transcription_base_is_versioned() {
    assert_eq!(DEFAULT_BASE, "https://api.mistral.ai/v1");
}

#[test]
fn the_form_carries_the_model_supported_and_provider_fields() {
    let optional = object(
        r#"{"language":"en","temperature":0.5,"timestamp_granularities":["word"],"response_format":"json","diarize":true,"keep":false,"count":3,"label":"x","unset":null,"model":"other","OPENAI_TRANSCRIPTION_PARAMS":"internal"}"#,
    );
    let request = transcription_request("voxtral", &audio(), &optional).expect("a form");
    assert_eq!(
        request.form,
        object(
            r#"{"model":"voxtral","language":"en","temperature":0.5,"timestamp_granularities":["word"],"response_format":"json","diarize":"true","keep":"false","count":"3","label":"x"}"#
        )
    );
    assert_eq!(request.file, audio());
}

#[rstest]
#[case::array(r#"{"diarize":[1,2]}"#, "diarize")]
#[case::object_value(r#"{"diarize":{"a":1}}"#, "diarize")]
fn a_provider_extra_that_cannot_be_a_form_value_names_itself(
    #[case] optional: &str,
    #[case] field: &str,
) {
    let error = transcription_request("voxtral", &audio(), &object(optional))
        .expect_err("a list cannot be a form value");
    assert!(matches!(&error, MistralAudioError::FormValue { field: named } if named == field));
    assert_eq!(error.status(), 400);
    assert!(error.to_string().contains(field));
}

#[test]
fn decodes_the_text_segments_and_language() {
    let raw = response(
        200,
        &[("Content-Type", "application/json")],
        br#"{"text":"hola","segments":[{"id":1,"text":"hola"}],"language":"es"}"#,
    );
    let decoded = decode_voxtral_transcript(&raw).expect("the reply decodes");
    assert_eq!(
        decoded.response,
        TranscriptionResponse {
            text: Some(String::from("hola")),
            extra: object(r#"{"segments":[{"id":1,"text":"hola"}],"language":"es"}"#),
        }
    );
    assert_eq!(
        decoded.hidden,
        object(r#"{"text":"hola","segments":[{"id":1,"text":"hola"}],"language":"es"}"#)
    );
}

#[rstest]
#[case::absent("{}", Some(""), "{}")]
#[case::null(r#"{"text":null}"#, Some(""), "{}")]
#[case::empty(r#"{"text":""}"#, Some(""), "{}")]
fn an_absent_or_null_text_is_empty(
    #[case] body: &str,
    #[case] text: Option<&str>,
    #[case] extra: &str,
) {
    let decoded =
        decode_voxtral_transcript(&response(200, &[], body.as_bytes())).expect("the reply decodes");
    assert_eq!(decoded.response.text.as_deref(), text);
    assert_eq!(decoded.response.extra, object(extra));
}

#[rstest]
#[case::absent(r#"{"text":"hi"}"#)]
#[case::null(r#"{"text":"hi","segments":null,"language":null}"#)]
fn preserves_the_optional_fields_by_presence(#[case] body: &str) {
    let decoded =
        decode_voxtral_transcript(&response(200, &[], body.as_bytes())).expect("the reply decodes");
    let segments = decoded.hidden.get("segments");
    let language = decoded.hidden.get("language");
    assert_eq!(
        decoded.response.extra.contains_key("segments"),
        segments.is_some()
    );
    assert_eq!(
        decoded.response.extra.contains_key("language"),
        language.is_some()
    );
}

#[rstest]
#[case::number(r#"{"text":5}"#)]
#[case::flag(r#"{"text":true}"#)]
#[case::list(r#"{"text":["hi"]}"#)]
fn rejects_a_text_that_is_not_a_string(#[case] body: &str) {
    let error = decode_voxtral_transcript(&response(200, &[], body.as_bytes()))
        .expect_err("the text is not a string");
    assert!(matches!(error, MistralAudioError::TextNotString));
    assert_eq!(error.status(), 500);
}

#[rstest]
#[case::non_json(429, b"slow".as_slice(), 429)]
#[case::malformed(502, br#"{"text":"#.as_slice(), 502)]
fn keeps_the_upstream_status_for_a_body_that_is_not_json(
    #[case] status: u16,
    #[case] body: &[u8],
    #[case] expected: u16,
) {
    let error = decode_voxtral_transcript(&response(status, &[], body))
        .expect_err("the body does not decode");
    assert!(
        matches!(error, MistralAudioError::ReplyNotJson { status: raised } if raised == expected)
    );
}

#[rstest]
#[case::list(b"[]".as_slice())]
#[case::text(br#""x""#.as_slice())]
#[case::number(b"7".as_slice())]
fn rejects_a_transcription_reply_that_is_not_an_object(#[case] body: &[u8]) {
    let error = decode_voxtral_transcript(&response(200, &[], body))
        .expect_err("the body is not an object");
    assert!(matches!(error, MistralAudioError::ReplyNotObject));
    assert_eq!(error.status(), 500);
}

#[test]
fn rejects_a_reply_larger_than_the_limit() {
    let oversized = vec![b'a'; REPLY_LIMIT + 1];
    let error = decode_voxtral_transcript(&response(200, &[], &oversized))
        .expect_err("the reply is too large");
    assert!(matches!(error, MistralAudioError::ReplyTooLarge { limit } if limit == REPLY_LIMIT));
}
