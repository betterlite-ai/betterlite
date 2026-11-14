#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};

use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioFileError, AudioTuple, AuthorizedAudioFiles, TupleContent,
};
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::{Headers, Object};
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::error::DeepgramTranscriptionError;
use super::request::{DeepgramTranscriptionConfig, map_openai_params, raw_audio_request};
use super::response::{
    DeepgramTranscription, REPLY_MAX_BYTES, TranscriptionTask, Word, decode_deepgram_transcript,
    reconstruct_diarized_text,
};
use crate::api::deepgram::common::DeepgramException;

const REPLY_BODY: &str = r#"{"metadata": {"duration": 12.5}, "results": {"channels": [{"alternatives": [ALTERNATIVE]}]}}"#;

const SPOKEN_WORD: &str = r#"{"word": "WORD", "start": 0.5, "end": 1.5, "speaker": SPEAKER}"#;

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::other("no file is authorized"))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::other("no file is authorized"))
    }
}

fn config(api_key: Option<&str>) -> DeepgramTranscriptionConfig {
    DeepgramTranscriptionConfig {
        api_key: api_key.map(SecretString::from),
        api_base: "https://api.deepgram.com/v1".to_owned(),
    }
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the text is a JSON object")
}

fn url(api_base: Option<&str>, optional_params: &str) -> String {
    config(None).complete_url(api_base, "nova-3", &object(optional_params))
}

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn deepgram_transcription_settings_default_to_the_hosted_base_and_no_key() {
    let settings = DeepgramTranscriptionConfig::from_environment(&environment(&[]));

    assert!(settings.api_key.is_none());
    assert_eq!(settings.api_base, "https://api.deepgram.com/v1");
}

#[test]
fn deepgram_transcription_settings_read_the_environment() {
    let settings = DeepgramTranscriptionConfig::from_environment(&environment(&[
        ("DEEPGRAM_API_KEY", "server-key"),
        ("DEEPGRAM_API_BASE", "https://proxy.test/v1"),
    ]));

    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(settings.api_base, "https://proxy.test/v1");
}

#[test]
fn deepgram_transcription_settings_treat_empty_variables_as_unset() {
    let settings = DeepgramTranscriptionConfig::from_environment(&environment(&[
        ("DEEPGRAM_API_KEY", ""),
        ("DEEPGRAM_API_BASE", ""),
    ]));

    assert!(settings.api_key.is_none());
    assert_eq!(settings.api_base, "https://api.deepgram.com/v1");
}

#[test]
fn deepgram_transcription_settings_read_each_variable_alone() {
    let key_only =
        DeepgramTranscriptionConfig::from_environment(&environment(&[("DEEPGRAM_API_KEY", "k")]));
    let base_only = DeepgramTranscriptionConfig::from_environment(&environment(&[(
        "DEEPGRAM_API_BASE",
        "https://eu.test/v1",
    )]));

    assert_eq!(
        key_only.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("k")
    );
    assert_eq!(key_only.api_base, "https://api.deepgram.com/v1");
    assert!(base_only.api_key.is_none());
    assert_eq!(base_only.api_base, "https://eu.test/v1");
}

fn reply(alternative: &str) -> Vec<u8> {
    REPLY_BODY.replace("ALTERNATIVE", alternative).into_bytes()
}

fn decoded(alternative: &str) -> DeepgramTranscription {
    decode_deepgram_transcript(&reply(alternative)).expect("the reply decodes")
}

fn serialized(response: &DeepgramTranscription) -> Object {
    serde_json::from_str(&serde_json::to_string(response).expect("the response serializes"))
        .expect("the response is an object")
}

fn spoken(text: &str, speaker: Option<i64>) -> Word {
    let member = serde_json::to_string(&speaker).expect("a speaker serializes");
    serde_json::from_str(
        &SPOKEN_WORD
            .replace("WORD", text)
            .replace("SPEAKER", &member),
    )
    .expect("a word")
}

#[test]
fn request_body_is_the_raw_audio_bytes_with_no_files() {
    let request =
        raw_audio_request(AudioFile::Bytes(b"RIFFdata"), &NoFiles).expect("bytes are audio");

    assert_eq!(
        request.data,
        AudioTranscriptionData::Bytes(b"RIFFdata".to_vec())
    );
    assert_eq!(request.files, None);
    assert_eq!(request.content_type, None);
}

#[test]
fn request_body_of_a_tuple_is_its_content_alone() {
    let tuple = AudioTuple {
        filename: "call.mp3",
        content: Some(TupleContent::Bytes(b"ID3")),
    };
    let request =
        raw_audio_request(AudioFile::Tuple(tuple), &NoFiles).expect("a tuple holds audio");

    assert_eq!(request.data, AudioTranscriptionData::Bytes(b"ID3".to_vec()));
    assert_eq!(request.files, None);
}

#[test]
fn request_rejects_a_bare_string_like_the_audio_processor() {
    let error = raw_audio_request(AudioFile::Text("a.wav"), &NoFiles)
        .expect_err("a bare string is not audio");

    assert!(matches!(
        error,
        DeepgramTranscriptionError::Audio {
            source: AudioFileError::BareString
        }
    ));
}

#[test]
fn language_is_the_only_mapped_openai_param() {
    let non_default = object(
        r#"{"language": "de", "response_format": "json", "temperature": 0.2, "prompt": "hi"}"#,
    );
    let mapped = map_openai_params(&non_default, object(r#"{"diarize": true}"#));

    assert_eq!(mapped, object(r#"{"diarize": true, "language": "de"}"#));
}

#[test]
fn a_mapped_language_replaces_an_existing_one() {
    let mapped = map_openai_params(
        &object(r#"{"language": "de"}"#),
        object(r#"{"language": "en", "diarize": true}"#),
    );

    assert_eq!(mapped, object(r#"{"language": "de", "diarize": true}"#));
}

#[rstest]
#[case::default_base(None, "https://api.deepgram.com/v1/listen?model=nova-3")]
#[case::trailing_slashes(Some("https://x.test/v1///"), "https://x.test/v1/listen?model=nova-3")]
#[case::empty_base_suppresses_the_default(Some(""), "/listen?model=nova-3")]
#[case::only_slashes(Some("//"), "/listen?model=nova-3")]
fn url_takes_the_explicit_base_before_the_configured_one(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(url(api_base, "{}"), expected);
}

#[test]
fn query_booleans_are_lowercase_and_null_is_skipped() {
    let built = url(
        None,
        r#"{"diarize": true, "punctuate": false, "smart_format": null, "paragraphs": true}"#,
    );

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&diarize=true&paragraphs=true&punctuate=false"
    );
}

#[test]
fn query_keeps_false_zero_and_empty_values() {
    let built = url(None, r#"{"count": 0, "label": "", "flag": false}"#);

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&count=0&flag=false&label="
    );
}

#[test]
fn query_leaves_out_the_model_internal_and_supported_names() {
    let built = url(
        None,
        r#"{"model": "other", "OPENAI_TRANSCRIPTION_PARAMS": ["x"], "language": "en", "Model": "kept", "redact": "pci"}"#,
    );

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&Model=kept&redact=pci"
    );
}

#[test]
fn query_writes_values_as_compact_json_without_expanding_lists() {
    let built = url(None, r#"{"keywords": ["a", "b"], "alpha": 1.5, "beta": 7}"#);

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&alpha=1.5&beta=7&keywords=%5B%22a%22%2C%22b%22%5D"
    );
}

#[test]
fn query_encodes_like_python_quote_plus() {
    let built = url(None, r#"{"term": "a b&c=d*e~f/\u00e9"}"#);

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&term=a+b%26c%3Dd%2Ae~f%2F%C3%A9"
    );
}

#[test]
fn query_encodes_a_percent_sign_that_precedes_a_tilde_escape() {
    let built = url(None, r#"{"term": "%7E~"}"#);

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&term=%257E~"
    );
}

#[test]
fn query_encodes_the_model_name() {
    let built = config(None).complete_url(None, "a b/c", &Object::new());

    assert_eq!(built, "https://api.deepgram.com/v1/listen?model=a+b%2Fc");
}

#[rstest]
#[case::explicit_key_wins(Some("explicit"), Some("env"), "Token explicit")]
#[case::configured_key(None, Some("env"), "Token env")]
#[case::missing_key_prints_none(None, None, "Token None")]
#[case::explicit_empty_is_used_as_given(Some(""), Some("env"), "Token ")]
fn validate_environment_builds_the_token_header(
    #[case] explicit: Option<&str>,
    #[case] configured: Option<&str>,
    #[case] expected: &str,
) {
    let key = explicit.map(SecretString::from);
    let headers = config(configured).validate_environment(key.as_ref());

    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn get_error_class_keeps_the_status_message_and_headers() {
    let headers = Headers::from([
        ("content-type".to_owned(), "application/json".to_owned()),
        ("x-request-id".to_owned(), "req-1".to_owned()),
    ]);

    let DeepgramException {
        status_code,
        message,
        headers: raised,
    } = DeepgramTranscriptionConfig::get_error_class("invalid audio", 401, &headers);

    assert_eq!(status_code, 401);
    assert_eq!(message, "invalid audio");
    assert_eq!(
        raised,
        BTreeMap::from([
            (
                "content-type".to_owned(),
                Json::Text("application/json".to_owned())
            ),
            ("x-request-id".to_owned(), Json::Text("req-1".to_owned())),
        ])
    );
}

#[test]
fn plain_transcript_without_speakers_uses_the_transcript() {
    let response = decoded(
        r#"{
            "transcript": "hello world",
            "paragraphs": {"transcript": "ignored"},
            "words": [
                {"word": "hello", "start": 0.5, "end": 1.5},
                {"word": "world", "start": 2, "end": 3}
            ]
        }"#,
    );

    assert_eq!(response.text.as_deref(), Some("hello world"));
    assert_eq!(response.task, TranscriptionTask::Transcribe);
    assert_eq!(response.language, "en");
    assert_eq!(response.duration.as_f64(), Some(12.5_f64));
    assert_eq!(
        serde_json::to_string(&response.words).expect("the words serialize"),
        r#"[{"word":"hello","start":0.5,"end":1.5},{"word":"world","start":2,"end":3}]"#
    );
}

#[test]
fn a_reply_without_words_has_no_words_entry() {
    let response = decoded(r#"{"transcript": "hi"}"#);

    assert!(response.words.is_none());
    assert!(!serialized(&response).contains_key("words"));
}

#[test]
fn an_empty_words_list_is_kept_and_uses_the_transcript() {
    let response = decoded(r#"{"transcript": "hi", "words": []}"#);

    assert_eq!(response.text.as_deref(), Some("hi"));
    assert_eq!(
        serde_json::to_string(&response.words).expect("the words serialize"),
        "[]"
    );
}

#[test]
fn a_null_transcript_leaves_the_text_out() {
    let response = decoded(r#"{"transcript": null}"#);

    assert!(response.text.is_none());
    assert!(!serialized(&response).contains_key("text"));
}

#[test]
fn a_diarized_reply_prefers_the_paragraphs_transcript() {
    let response = decoded(
        r#"{
            "transcript": "plain",
            "paragraphs": {"transcript": "\nSpeaker 0: from paragraphs\n"},
            "words": [
                {"word": "hello", "start": 0.5, "end": 1.5, "speaker": 0},
                {"word": "there", "start": 0.5, "end": 1.5, "speaker": 1}
            ]
        }"#,
    );

    assert_eq!(
        response.text.as_deref(),
        Some("\nSpeaker 0: from paragraphs\n")
    );
}

#[test]
fn a_diarized_reply_without_paragraphs_rebuilds_the_runs() {
    let response = decoded(
        r#"{
            "transcript": "plain",
            "words": [
                {"word": "hi", "punctuated_word": "Hi,", "start": 0, "end": 1, "speaker": 0},
                {"word": "there", "start": 0.5, "end": 1.5, "speaker": 0},
                {"word": "hello", "start": 0.5, "end": 1.5, "speaker": 1},
                {"word": "back", "start": 0.5, "end": 1.5, "speaker": 0}
            ]
        }"#,
    );

    assert_eq!(
        response.text.as_deref(),
        Some("Speaker 0: Hi, there\nSpeaker 1: hello\n\nSpeaker 0: back\n")
    );
}

#[test]
fn the_speaker_is_stripped_from_the_public_words() {
    let response = decoded(
        r#"{
            "transcript": "x",
            "words": [{"word": "hi", "start": 1, "end": 2, "speaker": 3, "confidence": 0.9}]
        }"#,
    );

    assert_eq!(
        serde_json::to_string(&response.words).expect("the words serialize"),
        r#"[{"word":"hi","start":1,"end":2}]"#
    );
}

#[test]
fn a_null_speaker_on_the_first_word_turns_diarization_on() {
    let response = decoded(
        r#"{
            "transcript": "plain",
            "words": [
                {"word": "hi", "start": 1, "end": 2, "speaker": null},
                {"word": "you", "start": 2, "end": 3}
            ]
        }"#,
    );

    assert_eq!(response.text.as_deref(), Some("\nSpeaker None: hi you\n"));
}

#[test]
fn a_speaker_on_the_second_word_alone_does_not_turn_diarization_on() {
    let response = decoded(
        r#"{
            "transcript": "plain",
            "words": [
                {"word": "hi", "start": 1, "end": 2},
                {"word": "you", "start": 2, "end": 3, "speaker": 1}
            ]
        }"#,
    );

    assert_eq!(response.text.as_deref(), Some("plain"));
}

#[test]
fn language_is_the_detected_one_unless_it_is_empty_or_null() {
    let language = |detected: &str| {
        let body = r#"{"metadata": {"duration": 1}, "results": {"channels": [{"detected_language": LANGUAGE, "alternatives": [{"transcript": "t"}]}]}}"#
            .replace("LANGUAGE", detected);
        decode_deepgram_transcript(body.as_bytes())
            .expect("the reply decodes")
            .language
    };

    assert_eq!(language(r#""de""#), "de");
    assert_eq!(language(r#""""#), "en");
    assert_eq!(language("null"), "en");
}

#[test]
fn only_the_first_channel_and_alternative_are_read() {
    let body = r#"{
        "metadata": {"duration": 1},
        "results": {"channels": [
            {"alternatives": [{"transcript": "first"}, 7]},
            "not a channel"
        ]}
    }"#;
    let response = decode_deepgram_transcript(body.as_bytes()).expect("the reply decodes");

    assert_eq!(response.text.as_deref(), Some("first"));
}

#[test]
fn the_hidden_params_hold_the_complete_provider_payload() {
    let body = r#"{
        "metadata": {"duration": 1, "request_id": "r"},
        "results": {"channels": [{"alternatives": [{"transcript": "t", "confidence": 0.5}]}]},
        "extra": [1, 2]
    }"#;
    let response = decode_deepgram_transcript(body.as_bytes()).expect("the reply decodes");

    assert_eq!(response.hidden_params, object(body));
}

#[rstest]
#[case::not_an_object("[1]")]
#[case::no_results(r#"{"metadata": {"duration": 1}}"#)]
#[case::no_metadata(r#"{"results": {"channels": [{"alternatives": [{"transcript": "t"}]}]}}"#)]
#[case::no_duration(
    r#"{"metadata": {}, "results": {"channels": [{"alternatives": [{"transcript": "t"}]}]}}"#
)]
#[case::null_duration(r#"{"metadata": {"duration": null}, "results": {"channels": [{"alternatives": [{"transcript": "t"}]}]}}"#)]
#[case::no_channels(r#"{"metadata": {"duration": 1}, "results": {}}"#)]
#[case::empty_channels(r#"{"metadata": {"duration": 1}, "results": {"channels": []}}"#)]
#[case::null_channels(r#"{"metadata": {"duration": 1}, "results": {"channels": null}}"#)]
#[case::no_alternatives(r#"{"metadata": {"duration": 1}, "results": {"channels": [{}]}}"#)]
#[case::empty_alternatives(
    r#"{"metadata": {"duration": 1}, "results": {"channels": [{"alternatives": []}]}}"#
)]
#[case::no_transcript(
    r#"{"metadata": {"duration": 1}, "results": {"channels": [{"alternatives": [{}]}]}}"#
)]
fn a_reply_without_the_required_members_is_rejected(#[case] body: &str) {
    let error = decode_deepgram_transcript(body.as_bytes()).expect_err("the shape is incomplete");

    assert!(matches!(
        error,
        DeepgramTranscriptionError::Transform { .. }
    ));
}

#[rstest]
#[case::null_words(r#"{"transcript": "t", "words": null}"#)]
#[case::word_without_text(r#"{"transcript": "t", "words": [{"start": 1, "end": 2}]}"#)]
#[case::word_without_start(r#"{"transcript": "t", "words": [{"word": "a", "end": 2}]}"#)]
#[case::word_without_end(r#"{"transcript": "t", "words": [{"word": "a", "start": 2}]}"#)]
#[case::word_not_an_object(r#"{"transcript": "t", "words": ["a"]}"#)]
#[case::words_not_a_list(r#"{"transcript": "t", "words": {"word": "a"}}"#)]
#[case::diarized_null_paragraphs(r#"{"transcript": "t", "paragraphs": null, "words": [{"word": "a", "start": 0.5, "end": 1.5, "speaker": 0}]}"#)]
#[case::diarized_paragraphs_without_transcript(r#"{"transcript": "t", "paragraphs": {}, "words": [{"word": "a", "start": 0.5, "end": 1.5, "speaker": 0}]}"#)]
#[case::diarized_null_punctuated_word(
    r#"{"words": [{"word": "a", "punctuated_word": null, "start": 1, "end": 2, "speaker": 0}]}"#
)]
fn a_malformed_alternative_is_rejected(#[case] alternative: &str) {
    let error =
        decode_deepgram_transcript(&reply(alternative)).expect_err("the alternative is malformed");

    assert!(matches!(
        error,
        DeepgramTranscriptionError::Transform { .. }
    ));
}

#[test]
fn null_paragraphs_are_ignored_when_the_reply_is_not_diarized() {
    let response = decoded(r#"{"transcript": "plain", "paragraphs": null}"#);

    assert_eq!(response.text.as_deref(), Some("plain"));
}

#[test]
fn a_diarized_reply_does_not_need_a_transcript() {
    let response = decoded(r#"{"words": [{"word": "a", "start": 0.5, "end": 1.5, "speaker": 0}]}"#);

    assert_eq!(response.text.as_deref(), Some("\nSpeaker 0: a\n"));
}

#[test]
fn a_failure_carries_the_raw_body_text() {
    let error = decode_deepgram_transcript(b"not json {").expect_err("the body is not JSON");

    let message = error.to_string();
    assert!(message.starts_with("Error transforming Deepgram response: "));
    assert!(message.ends_with("\nResponse: not json {"));
}

#[test]
fn bad_utf8_bytes_fail_and_name_the_byte_count() {
    let error =
        decode_deepgram_transcript(&[0xff, 0xfe, b'{']).expect_err("the bytes are not UTF-8");

    assert!(
        error
            .to_string()
            .ends_with("\nResponse: 3 bytes that are not UTF-8")
    );
}

#[test]
fn a_reply_over_the_size_limit_is_refused_before_it_is_read() {
    let at_limit = vec![b' '; REPLY_MAX_BYTES];
    let over_limit = vec![b' '; REPLY_MAX_BYTES + 1];

    assert!(matches!(
        decode_deepgram_transcript(&at_limit),
        Err(DeepgramTranscriptionError::Transform { .. })
    ));
    assert!(matches!(
        decode_deepgram_transcript(&over_limit),
        Err(DeepgramTranscriptionError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES
        })
    ));
}

#[test]
fn reconstruction_of_no_words_is_empty() {
    assert_eq!(reconstruct_diarized_text(&[]).expect("no words"), "");
}

#[test]
fn reconstruction_of_one_run_has_a_leading_and_trailing_newline() {
    let text =
        reconstruct_diarized_text(&[spoken("a", Some(2)), spoken("b", Some(2))]).expect("one run");

    assert_eq!(text, "\nSpeaker 2: a b\n");
}

#[test]
fn reconstruction_separates_earlier_runs_by_one_newline_and_the_last_by_two() {
    let words = [
        spoken("a", Some(0)),
        spoken("b", Some(1)),
        spoken("c", Some(0)),
        spoken("d", Some(1)),
    ];

    assert_eq!(
        reconstruct_diarized_text(&words).expect("four runs"),
        "Speaker 0: a\nSpeaker 1: b\nSpeaker 0: c\n\nSpeaker 1: d\n"
    );
}

#[test]
fn reconstruction_prefers_the_punctuated_word_and_falls_back_to_the_word() {
    let document =
        r#"{"word": "hi", "punctuated_word": "Hi!", "start": 0, "end": 1, "speaker": 0}"#;
    let punctuated: Word = serde_json::from_str(document).expect("a word");

    assert_eq!(
        reconstruct_diarized_text(&[punctuated, spoken("you", Some(0))]).expect("a run"),
        "\nSpeaker 0: Hi! you\n"
    );
}

#[test]
fn reconstruction_treats_a_missing_speaker_like_a_null_one() {
    let bare: Word =
        serde_json::from_str(r#"{"word": "b", "start": 0, "end": 1}"#).expect("a word");

    assert_eq!(
        reconstruct_diarized_text(&[spoken("a", Some(1)), spoken("n", None), bare])
            .expect("two runs"),
        "Speaker 1: a\n\nSpeaker None: n b\n"
    );
}

fn corpus_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_owned()
}

#[rstest]
fn replays_the_deepgram_transcript_corpus(
    #[files("fuzz/corpus/deepgram_transcript/*")] path: PathBuf,
) {
    let name = corpus_name(&path);
    let bytes = fs::read(&path).expect("a corpus file is readable");
    let outcome = decode_deepgram_transcript(&bytes);

    if name.starts_with("rejected-") {
        assert!(matches!(
            outcome,
            Err(DeepgramTranscriptionError::Transform { .. })
        ));
        return;
    }
    let expected = match name.as_str() {
        "ok-diarized-paragraphs.json" => "\nSpeaker 0: Hello there.\n",
        "ok-diarized-words.json" => "Speaker 0: Hi,\nSpeaker 1: you\n\nSpeaker 0: ok\n",
        "ok-null-speaker.json" => "\nSpeaker None: x\n",
        "ok-plain.json" => "hello there",
        "ok-speaker-on-second-word.json" => "x",
        other => panic!("the seed {other} has no expected transcript"),
    };
    let transcript = outcome.expect("an ok seed decodes");
    assert_eq!(transcript.text.as_deref(), Some(expected));
}
