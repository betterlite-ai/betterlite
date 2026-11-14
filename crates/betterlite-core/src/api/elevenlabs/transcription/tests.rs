#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use crate::api::elevenlabs::common::ElevenLabsError;

use super::PROVIDER;
use super::auth::{TranscriptionKeys, validate_environment};
use super::endpoint::complete_url;
use super::error::TranscriptionError;
use super::mapping::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::{FILE_FIELD, number_text, transform_request};
use super::response::{
    ElevenTranscript, MAX_REPLY_BYTES, TranscriptTask, TranscriptWord, decode_eleven_transcript,
};

const COMPLETE_REPLY: &str = r#"{"text": "hello world", "language_code": "eng", "language_probability": 0.98, "words": [{"text": "hello", "start": 0.0, "end": 0.5, "type": "word", "logprob": -0.1}, {"text": " ", "start": 0.5, "end": 0.6, "type": "spacing"}, {"text": "(laughs)", "start": 0.6, "end": 0.7, "type": "audio_event"}, {"text": "world", "start": 0.7, "end": 1.2, "type": "word"}]}"#;

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn number(text: &str) -> Number {
    serde_json::from_str(text).expect("a JSON number")
}

fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"RIFF".to_vec(),
        filename: "clip.wav".to_owned(),
        content_type: "audio/wav".to_owned(),
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn transcript(body: &str) -> ElevenTranscript {
    decode_eleven_transcript(body.as_bytes(), MAX_REPLY_BYTES).expect("a decoded transcript")
}

fn word(text: &str, start: Number, end: Number) -> TranscriptWord {
    TranscriptWord {
        word: Some(text.to_owned()),
        start: Some(start),
        end: Some(end),
    }
}

fn form_fields(options: &str) -> Vec<(String, String)> {
    let form = transform_request("scribe_v1", &audio(), &object(options)).expect("a form");

    form.fields.into_iter().collect()
}

fn headers(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn the_provider_name_is_elevenlabs() {
    assert_eq!(PROVIDER, "elevenlabs");
}

#[test]
fn the_supported_openai_params_are_language_and_temperature() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["language", "temperature"]);
}

#[rstest]
#[case::language_becomes_language_code(r#"{"language": "en"}"#, r#"{"language_code": "en"}"#)]
#[case::temperature_is_copied(r#"{"temperature": 0}"#, r#"{"temperature": 0}"#)]
#[case::null_language_is_copied(r#"{"language": null}"#, r#"{"language_code": null}"#)]
#[case::other_openai_params_are_ignored(r#"{"prompt": "x", "response_format": "json"}"#, "{}")]
fn mapping_copies_only_the_supported_names(#[case] given: &str, #[case] expected: &str) {
    let mut optional = Object::new();

    map_openai_params(&object(given), &mut optional);

    assert_eq!(optional, object(expected));
}

#[test]
fn mapping_keeps_the_existing_options_and_overwrites_the_language_code() {
    let mut optional = object(r#"{"language_code": "fr", "temperature": 0.2, "custom": 1}"#);

    map_openai_params(&object(r#"{"language": "de"}"#), &mut optional);

    assert_eq!(
        optional,
        object(r#"{"language_code": "de", "temperature": 0.2, "custom": 1}"#)
    );
}

#[test]
fn the_form_starts_with_model_id_and_converts_supported_values() {
    let fields =
        form_fields(r#"{"language_code": "en", "temperature": 0.4, "seed": 3, "vad": true}"#);

    assert_eq!(
        fields,
        [
            ("model_id".to_owned(), "scribe_v1".to_owned()),
            ("temperature".to_owned(), "0.4".to_owned()),
            ("language_code".to_owned(), "en".to_owned()),
            ("seed".to_owned(), "3".to_owned()),
            ("vad".to_owned(), "True".to_owned()),
        ]
    );
}

#[test]
fn the_form_drops_reserved_null_and_false_values_the_way_litellm_does() {
    let fields = form_fields(
        r#"{"model": "other", "OPENAI_TRANSCRIPTION_PARAMS": {}, "diarize": false, "note": "hi", "score": 1e-05, "tags": null}"#,
    );

    assert_eq!(
        fields,
        [
            ("model_id".to_owned(), "scribe_v1".to_owned()),
            ("diarize".to_owned(), "False".to_owned()),
            ("note".to_owned(), "hi".to_owned()),
            ("score".to_owned(), "1e-05".to_owned()),
        ]
    );
}

#[test]
fn a_provider_model_id_replaces_the_model_name_in_place() {
    let fields = form_fields(r#"{"model_id": "override"}"#);

    assert_eq!(fields, [("model_id".to_owned(), "override".to_owned())]);
}

#[rstest]
#[case::array(r#"{"size": ["small"]}"#)]
#[case::object(r#"{"size": {"width": 1}}"#)]
fn a_form_value_that_is_not_a_scalar_names_its_field(#[case] options: &str) {
    let error =
        transform_request("scribe_v1", &audio(), &object(options)).expect_err("a form error");

    assert!(matches!(&error, TranscriptionError::FormValue { field } if field == "size"));
}

#[test]
fn the_request_carries_one_file_tuple_with_the_processed_audio() {
    let form = transform_request("scribe_v1", &audio(), &Object::new()).expect("a form");

    assert_eq!(form.file.name, FILE_FIELD);
    assert_eq!(form.file.file_name.as_deref(), Some("clip.wav"));
    assert_eq!(form.file.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(form.file.data, b"RIFF".to_vec());
}

#[rstest]
#[case::environment(
    None,
    "https://api.elevenlabs.io",
    "https://api.elevenlabs.io/v1/speech-to-text"
)]
#[case::environment_slashes(None, "https://env.test//", "https://env.test/v1/speech-to-text")]
#[case::explicit(
    Some("https://proxy.test/"),
    "https://env.test",
    "https://proxy.test/v1/speech-to-text"
)]
#[case::empty_explicit(Some(""), "https://env.test", "/v1/speech-to-text")]
fn the_url_prefers_an_explicit_base_even_when_it_is_empty(
    #[case] api_base: Option<&str>,
    #[case] environment: &str,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(api_base, environment), expected);
}

#[rstest]
#[case::integer("7", "7")]
#[case::negative_zero("-0", "0")]
#[case::fraction("0.4", "0.4")]
#[case::integral_float("3.0", "3.0")]
#[case::small_exponent("1e-05", "1e-05")]
#[case::negative_exponent("1.5e-7", "1.5e-07")]
#[case::large_exponent("1e+20", "1e+20")]
#[case::sixteen_digits("1e16", "1e+16")]
#[case::fifteen_digits("1e15", "1000000000000000.0")]
#[case::small_fraction("0.0001", "0.0001")]
#[case::big_integer("123456789012345678901234567890", "123456789012345678901234567890")]
fn numbers_are_rendered_with_pythons_str_spelling(#[case] literal: &str, #[case] expected: &str) {
    assert_eq!(number_text(&number(literal)), expected);
}

#[rstest]
#[case::explicit_first(Some("a"), Some("b"), "a")]
#[case::environment_second(None, Some("b"), "b")]
#[case::empty_explicit_falls_through(Some(""), Some("b"), "b")]
#[case::empty_environment_value_is_accepted(Some(""), Some(""), "")]
#[case::false_text_is_a_key(Some("false"), Some("b"), "false")]
fn the_key_falls_from_the_explicit_value_to_the_environment(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let explicit_key = explicit.map(secret);
    let environment_key = environment.map(secret);

    let updated = validate_environment(
        &IndexMap::new(),
        &TranscriptionKeys {
            explicit: explicit_key.as_ref(),
            environment: environment_key.as_ref(),
        },
    )
    .expect("validated headers");

    assert_eq!(
        updated.get("xi-api-key").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn a_missing_key_names_the_environment_variable() {
    let error = validate_environment(
        &IndexMap::new(),
        &TranscriptionKeys {
            explicit: None,
            environment: None,
        },
    )
    .expect_err("a missing key");

    assert_eq!(
        error.to_string(),
        "ElevenLabs API key is required. Set ELEVENLABS_API_KEY environment variable."
    );
}

#[test]
fn validating_the_environment_overwrites_the_auth_header_and_keeps_the_rest() {
    let key = secret("k");
    let given = headers(&[("Accept", "application/json"), ("xi-api-key", "old")]);

    let updated = validate_environment(
        &given,
        &TranscriptionKeys {
            explicit: Some(&key),
            environment: None,
        },
    )
    .expect("validated headers");

    assert_eq!(
        updated,
        headers(&[("Accept", "application/json"), ("xi-api-key", "k")])
    );
    assert_eq!(given.get("xi-api-key").map(String::as_str), Some("old"));
}

#[test]
fn the_provider_error_keeps_the_message_status_and_headers() {
    let provider_headers = headers(&[("Content-Type", "application/json")]);

    let error = ElevenLabsError {
        status_code: 401,
        message: "bad key".to_owned(),
        headers: provider_headers.clone(),
    };

    assert_eq!(error.status_code, 401);
    assert_eq!(error.message, "bad key");
    assert_eq!(error.to_string(), "bad key");
    assert_eq!(error.headers, provider_headers);
}

#[test]
fn a_complete_reply_projects_the_words_and_keeps_the_provider_answer_hidden() {
    let reply = transcript(COMPLETE_REPLY);

    assert_eq!(reply.text.as_deref(), Some("hello world"));
    assert_eq!(reply.task, TranscriptTask::Transcribe);
    assert_eq!(reply.language.as_deref(), Some("eng"));
    assert_eq!(
        reply.words,
        Some(vec![
            word("hello", number("0.0"), number("0.5")),
            word("world", number("0.7"), number("1.2")),
        ])
    );
    assert_eq!(reply.hidden, object(COMPLETE_REPLY));
}

#[test]
fn the_transcript_serializes_the_hidden_answer_out_of_the_response() {
    let reply = transcript(
        r#"{"text": "hi", "language_code": "eng", "words": [{"text": "hi", "start": 0, "end": 1, "type": "word"}]}"#,
    );

    let rendered = serde_json::to_value(&reply).expect("a serialized transcript");

    assert_eq!(
        rendered,
        serde_json::json!({
            "text": "hi",
            "task": "transcribe",
            "language": "eng",
            "words": [{"word": "hi", "start": 0, "end": 1}],
        })
    );
}

#[test]
fn the_transcript_keeps_present_nulls_on_the_wire() {
    let reply = transcript(
        r#"{"text": null, "language_code": null, "words": [{"type": "word", "text": null, "start": null, "end": null}]}"#,
    );

    let rendered = serde_json::to_value(&reply).expect("a serialized transcript");

    assert_eq!(
        rendered,
        serde_json::json!({
            "text": null,
            "task": "transcribe",
            "language": null,
            "words": [{"word": null, "start": null, "end": null}],
        })
    );
}

#[rstest]
#[case::absent("{}", None)]
#[case::empty(r#"{"words": []}"#, Some(vec![]))]
#[case::spacing_only(r#"{"words": [{"type": "spacing", "text": " "}]}"#, Some(vec![]))]
#[case::events_only(
    r#"{"words": [{"type": "audio_event", "text": "(laughs)"}]}"#,
    Some(vec![])
)]
#[case::non_string_types(
    r#"{"words": [{"type": 5}, {"type": true}, {"type": null}, {"type": ["word"]}, {"text": "no type"}]}"#,
    Some(vec![])
)]
fn words_are_present_exactly_when_the_provider_sends_the_field(
    #[case] body: &str,
    #[case] expected: Option<Vec<TranscriptWord>>,
) {
    assert_eq!(transcript(body).words, expected);
}

#[rstest]
#[case::empty_document("{}", Some(""), Some("unknown"))]
#[case::explicit_nulls(r#"{"text": null, "language_code": null}"#, None, None)]
#[case::null_text_only(r#"{"text": null, "language_code": "eng"}"#, None, Some("eng"))]
#[case::null_language_only(r#"{"text": "hi", "language_code": null}"#, Some("hi"), None)]
fn absent_text_and_language_take_the_source_defaults_and_nulls_stay_null(
    #[case] body: &str,
    #[case] text: Option<&str>,
    #[case] language: Option<&str>,
) {
    let reply = transcript(body);

    assert_eq!(reply.text.as_deref(), text);
    assert_eq!(reply.language.as_deref(), language);
}

#[rstest]
#[case::missing_text_and_bounds(
    r#"{"type": "word", "start": 0.5}"#,
    Some(""),
    Some("0.5"),
    Some("0")
)]
#[case::null_text_and_end(
    r#"{"type": "word", "text": null, "start": 1, "end": null}"#,
    None,
    Some("1"),
    None
)]
#[case::null_start_and_missing_end(
    r#"{"type": "word", "text": "hi", "start": null}"#,
    Some("hi"),
    None,
    Some("0")
)]
fn words_take_empty_and_zero_defaults_only_when_the_field_is_absent(
    #[case] entry: &str,
    #[case] text: Option<&str>,
    #[case] start: Option<&str>,
    #[case] end: Option<&str>,
) {
    let reply = transcript(&format!(r#"{{"words": [{entry}]}}"#));

    let found = reply
        .words
        .and_then(|words| words.into_iter().next())
        .expect("one word");

    assert_eq!(found.word.as_deref(), text);
    assert_eq!(
        found.start.as_ref().map(Number::to_string).as_deref(),
        start
    );
    assert_eq!(found.end.as_ref().map(Number::to_string).as_deref(), end);
}

#[rstest]
#[case::invalid_json("not json")]
#[case::array_body("[]")]
#[case::trailing_bytes("{} {}")]
#[case::words_not_a_list(r#"{"words": "hi"}"#)]
#[case::words_null(r#"{"words": null}"#)]
#[case::word_not_an_object(r#"{"words": [1]}"#)]
#[case::text_not_a_string(r#"{"text": 5}"#)]
#[case::start_not_a_number(r#"{"words": [{"type": "word", "start": "0"}]}"#)]
fn malformed_replies_fail_with_the_raw_response_text(#[case] body: &str) {
    let error =
        decode_eleven_transcript(body.as_bytes(), MAX_REPLY_BYTES).expect_err("a decode error");

    match error {
        TranscriptionError::Reply { raw, .. } => assert_eq!(raw, body),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn a_reply_that_is_not_utf8_reports_its_length() {
    let error =
        decode_eleven_transcript(&[0xff, 0xfe], MAX_REPLY_BYTES).expect_err("a decode error");

    assert!(
        error
            .to_string()
            .ends_with("\nResponse: 2 bytes that are not UTF-8")
    );
}

#[test]
fn a_reply_at_the_limit_decodes_and_one_byte_over_is_an_error() {
    let body = br#"{"text": "hi"}"#;

    let reply = decode_eleven_transcript(body, body.len()).expect("a reply at the limit");
    let error = decode_eleven_transcript(body, body.len() - 1).expect_err("an over-limit reply");

    assert_eq!(reply.text.as_deref(), Some("hi"));
    assert!(matches!(
        error,
        TranscriptionError::ReplyTooLarge { limit } if limit == body.len() - 1
    ));
}

#[rstest]
fn replays_the_eleven_transcript_corpus(#[files("fuzz/corpus/eleven_transcript/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus file");

    drop(decode_eleven_transcript(&bytes, MAX_REPLY_BYTES));
}
