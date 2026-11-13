use betterlite_config::sections::mistral_resources::Settings;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::super::super::error::MistralAudioError;
use super::super::super::reply::REPLY_LIMIT;
use super::super::super::speech::{
    DEFAULT_BASE, OPENAI_PARAMS, complete_url, decode_voxtral_reply, map_openai_params,
    request_body, validate_environment,
};
use super::super::{object, response, speech_request};
use crate::api::speech_provider_contract::SpeechVoice;

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

fn name(text: &str) -> SpeechVoice {
    SpeechVoice::Name(text.to_owned())
}

fn options(text: &str) -> SpeechVoice {
    SpeechVoice::Options(object(text))
}

#[test]
fn declares_the_two_openai_params() {
    assert_eq!(OPENAI_PARAMS, ["voice", "response_format"]);
}

#[rstest]
#[case::alias(Some(name("alloy")), "{}", Some("en_paul_neutral"))]
#[case::alias_in_upper_case(Some(name("ECHO")), "{}", Some("gb_oliver_neutral"))]
#[case::alias_with_padding(Some(name("  fable  ")), "{}", Some("en_paul_cheerful"))]
#[case::unknown_keeps_case(Some(name("MyVoice")), "{}", Some("MyVoice"))]
#[case::blank_name(Some(name("   ")), "{}", None)]
#[case::options_voice_id(
    Some(options(r#"{"voice_id":"onyx"}"#)),
    "{}",
    Some("en_paul_confident")
)]
#[case::options_id(Some(options(r#"{"id":"Nova"}"#)), "{}", Some("gb_jane_sarcasm"))]
#[case::options_name(Some(options(r#"{"name":"shimmer"}"#)), "{}", Some("gb_jane_sarcasm"))]
#[case::options_priority(
    Some(options(r#"{"voice_id":"alloy","id":"echo"}"#)),
    "{}",
    Some("en_paul_neutral")
)]
#[case::options_skip_non_string(
    Some(options(r#"{"voice_id":5,"id":"echo"}"#)),
    "{}",
    Some("gb_oliver_neutral")
)]
#[case::options_skip_blank(Some(options(r#"{"voice_id":"","id":"  ","name":""}"#)), "{}", None)]
#[case::kwargs_voice(None, r#"{"voice_id":"alloy"}"#, Some("en_paul_neutral"))]
#[case::kwargs_after_blank(Some(name("  ")), r#"{"voice_id":"nova"}"#, Some("gb_jane_sarcasm"))]
#[case::voice_wins(
    Some(name("shimmer")),
    r#"{"voice_id":"alloy"}"#,
    Some("gb_jane_sarcasm")
)]
#[case::kwargs_non_string(None, r#"{"voice_id":7}"#, None)]
#[case::absent(None, "{}", None)]
fn resolves_the_voice(
    #[case] voice: Option<SpeechVoice>,
    #[case] kwargs: &str,
    #[case] expected: Option<&str>,
) {
    let (mapped, params) = map_openai_params(&object("{}"), voice.as_ref(), &object(kwargs));
    assert_eq!(mapped.as_deref(), expected);
    assert!(params.is_empty());
}

#[rstest]
#[case::both(
    r#"{"response_format":"wav"}"#,
    r#"{"ref_audio":"data:audio/wav;base64,AA"}"#,
    r#"{"response_format":"wav","ref_audio":"data:audio/wav;base64,AA"}"#
)]
#[case::empty_strings(
    r#"{"response_format":""}"#,
    r#"{"ref_audio":""}"#,
    r#"{"response_format":"","ref_audio":""}"#
)]
#[case::non_strings(r#"{"response_format":5}"#, r#"{"ref_audio":{"a":1}}"#, "{}")]
#[case::each_from_its_own_source(
    r#"{"ref_audio":"optional"}"#,
    r#"{"response_format":"wav"}"#,
    "{}"
)]
#[case::other_params(r#"{"speed":2}"#, "{}", "{}")]
fn keeps_only_the_string_reply_options(
    #[case] optional_params: &str,
    #[case] kwargs: &str,
    #[case] expected: &str,
) {
    let (_, params) = map_openai_params(&object(optional_params), None, &object(kwargs));
    assert_eq!(params, object(expected));
}

#[rstest]
#[case::explicit(Some("explicit"), None, "Bearer explicit")]
#[case::environment(None, Some("environment"), "Bearer environment")]
#[case::explicit_wins(Some("explicit"), Some("environment"), "Bearer explicit")]
fn adds_the_authorization_header(
    #[case] api_key: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let key = api_key.map(SecretString::from);
    let resolved = validate_environment(&headers(&[]), key.as_ref(), &settings(environment))
        .expect("a key resolves");
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some(expected)
    );
    assert_eq!(
        resolved.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn a_speech_call_without_a_key_fails_with_401() {
    let error =
        validate_environment(&headers(&[]), None, &settings(None)).expect_err("no key fails");
    assert!(matches!(error, MistralAudioError::MissingApiKey));
    assert_eq!(error.status(), 401);
    assert!(error.to_string().contains("MISTRAL_API_KEY"));
}

#[test]
fn replaces_the_authorization_and_content_type_headers() {
    let given = headers(&[
        ("authorization", "Bearer lower"),
        ("Authorization", "Bearer old"),
        ("content-type", "text/plain"),
        ("X-Trace", "1"),
    ]);
    let key = SecretString::from("key");
    let resolved =
        validate_environment(&given, Some(&key), &settings(None)).expect("a key resolves");
    assert_eq!(
        resolved,
        headers(&[
            ("authorization", "Bearer lower"),
            ("Authorization", "Bearer key"),
            ("content-type", "text/plain"),
            ("X-Trace", "1"),
            ("Content-Type", "application/json"),
        ])
    );
}

#[rstest]
#[case::default(None, "https://api.mistral.ai/v1/audio/speech")]
#[case::empty(Some(""), "/v1/audio/speech")]
#[case::unversioned(Some("https://mock"), "https://mock/v1/audio/speech")]
#[case::trailing_slash(Some("https://mock/"), "https://mock/v1/audio/speech")]
#[case::versioned(Some("https://mock/v1"), "https://mock/v1/audio/speech")]
#[case::versioned_slash(Some("https://mock/v1/"), "https://mock/v1/audio/speech")]
#[case::doubled_version(Some("https://mock/v1/v1"), "https://mock/v1/v1/audio/speech")]
fn builds_the_speech_url(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(complete_url(api_base), expected);
}

#[test]
fn the_default_speech_base_is_versioned() {
    assert_eq!(DEFAULT_BASE, "https://api.mistral.ai/v1");
}

#[test]
fn the_request_body_carries_the_mapped_values() {
    let optional = object(r#"{"response_format":"wav","ref_audio":"data:audio/wav;base64,AA"}"#);
    let request = request_body("voxtral", "hello", Some("en_paul_neutral"), &optional);
    assert_eq!(
        request.dict_body,
        Some(object(
            r#"{"model":"voxtral","input":"hello","voice_id":"en_paul_neutral","response_format":"wav","ref_audio":"data:audio/wav;base64,AA"}"#
        ))
    );
    assert_eq!(
        request.headers,
        Some(headers(&[("Content-Type", "application/json")]))
    );
    assert!(request.ssml_body.is_none());
}

#[test]
fn the_request_body_omits_an_absent_voice_and_non_string_options() {
    let optional = object(r#"{"response_format":5,"ref_audio":null}"#);
    let request = request_body("voxtral", "hello", None, &optional);
    assert_eq!(
        request.dict_body,
        Some(object(r#"{"model":"voxtral","input":"hello"}"#))
    );
}

#[test]
fn the_request_body_keeps_an_explicit_empty_voice() {
    let request = request_body("voxtral", "hello", Some(""), &object("{}"));
    assert_eq!(
        request.dict_body,
        Some(object(
            r#"{"model":"voxtral","input":"hello","voice_id":""}"#
        ))
    );
}

const AUDIO: &str = "aGVsbG8=";

#[test]
fn decodes_the_audio_and_rebuilds_its_headers() {
    let raw = response(
        502,
        &[
            ("Content-Encoding", "gzip"),
            ("Transfer-Encoding", "chunked"),
            ("Content-Length", "99"),
            ("Content-Type", "application/json"),
            ("X-Keep", "1"),
            ("X-Dup", "a"),
            ("X-Dup", "b"),
        ],
        format!(r#"{{"audio_data":"{AUDIO}"}}"#).as_bytes(),
    );
    let content = decode_voxtral_reply(&speech_request(&[("response_format", "wav")]), &raw)
        .expect("the reply decodes");
    assert_eq!(content.status, 200);
    assert_eq!(content.body, b"hello");
    assert_eq!(
        content.headers,
        headers(&[
            ("x-keep", "1"),
            ("x-dup", "a, b"),
            ("content-length", "5"),
            ("content-type", "audio/wav"),
        ])
    );
}

#[rstest]
#[case::mp3("mp3", "audio/mpeg")]
#[case::wav("wav", "audio/wav")]
#[case::pcm("pcm", "audio/pcm")]
#[case::flac("flac", "audio/flac")]
#[case::opus("opus", "audio/ogg")]
#[case::unknown("mp4", "audio/mpeg")]
#[case::upper_case("MP3", "audio/mpeg")]
fn picks_the_content_type_from_the_requested_format(#[case] format: &str, #[case] expected: &str) {
    let raw = response(
        200,
        &[],
        format!(r#"{{"audio_data":"{AUDIO}"}}"#).as_bytes(),
    );
    let content = decode_voxtral_reply(&speech_request(&[("response_format", format)]), &raw)
        .expect("the reply decodes");
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn defaults_the_content_type_when_the_request_names_no_format() {
    let raw = response(
        200,
        &[],
        format!(r#"{{"audio_data":"{AUDIO}"}}"#).as_bytes(),
    );
    let content = decode_voxtral_reply(&speech_request(&[]), &raw).expect("the reply decodes");
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some("audio/mpeg")
    );
}

#[rstest]
#[case::non_json(404, b"nope".as_slice(), 404)]
#[case::malformed(502, br#"{"audio_data":"#.as_slice(), 502)]
#[case::invalid_utf8(429, b"{\"audio_data\":\"\xff\"}".as_slice(), 429)]
fn keeps_the_upstream_status_for_a_body_that_is_not_json(
    #[case] status: u16,
    #[case] body: &[u8],
    #[case] expected: u16,
) {
    let error = decode_voxtral_reply(&speech_request(&[]), &response(status, &[], body))
        .expect_err("the body does not decode");
    assert!(
        matches!(error, MistralAudioError::ReplyNotJson { status: raised } if raised == expected)
    );
    assert_eq!(error.status(), expected);
}

#[rstest]
#[case::non_object(b"[]".as_slice(), 500)]
#[case::text_payload(br#""x""#.as_slice(), 500)]
#[case::no_audio_data(br#"{"text":"hi"}"#.as_slice(), 500)]
#[case::null_audio_data(br#"{"audio_data":null}"#.as_slice(), 500)]
#[case::empty_audio_data(br#"{"audio_data":""}"#.as_slice(), 500)]
#[case::number_audio_data(br#"{"audio_data":5}"#.as_slice(), 500)]
#[case::invalid_base64(br#"{"audio_data":"@@@"}"#.as_slice(), 500)]
#[case::bad_padding(br#"{"audio_data":"aGk=="}"#.as_slice(), 500)]
fn rejects_a_speech_reply_without_usable_audio(#[case] body: &[u8], #[case] expected: u16) {
    let error = decode_voxtral_reply(&speech_request(&[]), &response(200, &[], body))
        .expect_err("the reply carries no audio");
    assert_eq!(error.status(), expected);
}

#[test]
fn accepts_the_trailing_bits_python_accepts() {
    let raw = response(200, &[], br#"{"audio_data":"aGl="}"#);
    let content = decode_voxtral_reply(&speech_request(&[]), &raw).expect("the reply decodes");
    assert_eq!(content.body, b"hi");
}

#[test]
fn rejects_a_reply_larger_than_the_limit() {
    let oversized = vec![b'a'; REPLY_LIMIT + 1];
    let raw = response(200, &[], &oversized);
    let error =
        decode_voxtral_reply(&speech_request(&[]), &raw).expect_err("the reply is too large");
    assert!(matches!(error, MistralAudioError::ReplyTooLarge { limit } if limit == REPLY_LIMIT));
    assert_eq!(error.status(), 500);
}

#[test]
fn reads_a_reply_of_exactly_sixteen_mebibytes() {
    assert_eq!(REPLY_LIMIT, 16_777_216);
    let at_limit = vec![b'a'; REPLY_LIMIT];
    let error = decode_voxtral_reply(&speech_request(&[]), &response(200, &[], &at_limit))
        .expect_err("the body is not JSON");
    assert!(matches!(
        error,
        MistralAudioError::ReplyNotJson { status: 200 }
    ));
}

#[test]
fn the_decoder_keeps_a_null_payload_shape_typed() {
    let raw = response(200, &[], br#"{"audio_data":"aGVsbG8=","extra":{"n":null}}"#);
    let content = decode_voxtral_reply(&speech_request(&[]), &raw).expect("the reply decodes");
    assert_eq!(content.body, b"hello");
}
