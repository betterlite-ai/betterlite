#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};

use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioFileError, AudioTuple, AuthorizedAudioFiles, TupleContent,
};
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::error::DeepgramTranscriptionError;
use super::request::{
    DeepgramTranscriptionConfig, map_openai_params, transform_audio_transcription_request,
};
use super::response::{
    REPLY_MAX_BYTES, Word, decode_deepgram_transcript, reconstruct_diarized_transcript,
};
use crate::api::deepgram::common::DeepgramException;

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

fn params(value: &Value) -> IndexMap<String, Value> {
    serde_json::from_value(value.clone()).expect("the params are an object")
}

fn url(api_base: Option<&str>, optional_params: &Value) -> String {
    config(None).complete_url(api_base, "nova-3", &params(optional_params))
}

fn reply(alternative: &Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "metadata": {"duration": 12.5},
        "results": {"channels": [{"alternatives": [alternative]}]},
    }))
    .expect("the reply serializes")
}

fn decoded(alternative: &Value) -> Value {
    let response = decode_deepgram_transcript(&reply(alternative)).expect("the reply decodes");
    serde_json::to_value(response).expect("the response serializes")
}

fn word(text: &str, speaker: Option<i64>) -> Value {
    json!({"word": text, "start": 0.5, "end": 1.5, "speaker": speaker})
}

#[test]
fn request_body_is_the_raw_audio_bytes_with_no_files() {
    let request = transform_audio_transcription_request(AudioFile::Bytes(b"RIFFdata"), &NoFiles)
        .expect("bytes are audio");

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
    let request = transform_audio_transcription_request(AudioFile::Tuple(tuple), &NoFiles)
        .expect("a tuple holds audio");

    assert_eq!(request.data, AudioTranscriptionData::Bytes(b"ID3".to_vec()));
    assert_eq!(request.files, None);
}

#[test]
fn request_rejects_a_bare_string_like_the_audio_processor() {
    let error = transform_audio_transcription_request(AudioFile::Text("a.wav"), &NoFiles)
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
    let non_default = params(&json!({
        "language": "de",
        "response_format": "json",
        "temperature": 0.2,
        "prompt": "hi",
    }));
    let mapped = map_openai_params(&non_default, params(&json!({"diarize": true})));

    assert_eq!(
        serde_json::to_value(&mapped).expect("params serialize"),
        json!({"diarize": true, "language": "de"})
    );
}

#[test]
fn a_mapped_language_replaces_an_existing_one_in_place() {
    let mapped = map_openai_params(
        &params(&json!({"language": "de"})),
        params(&json!({"language": "en", "diarize": true})),
    );

    assert_eq!(
        mapped.keys().map(String::as_str).collect::<Vec<_>>(),
        ["language", "diarize"]
    );
    assert_eq!(mapped.get("language"), Some(&json!("de")));
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
    assert_eq!(url(api_base, &json!({})), expected);
}

#[test]
fn query_booleans_are_lowercase_and_null_is_skipped() {
    let built = url(
        None,
        &json!({"diarize": true, "punctuate": false, "smart_format": null, "paragraphs": true}),
    );

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&diarize=true&punctuate=false&paragraphs=true"
    );
}

#[test]
fn query_keeps_false_zero_and_empty_values() {
    let built = url(None, &json!({"count": 0, "label": "", "flag": false}));

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&count=0&label=&flag=false"
    );
}

#[test]
fn query_leaves_out_the_model_internal_and_supported_names() {
    let built = url(
        None,
        &json!({
            "model": "other",
            "OPENAI_TRANSCRIPTION_PARAMS": ["x"],
            "language": "en",
            "Model": "kept",
            "redact": "pci",
        }),
    );

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&Model=kept&redact=pci"
    );
}

#[test]
fn query_follows_the_insertion_order_and_does_not_expand_lists() {
    let built = url(
        None,
        &json!({"keywords": ["a", "b"], "alpha": 1.5, "beta": 7}),
    );

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&keywords=%5B%22a%22%2C%22b%22%5D&alpha=1.5&beta=7"
    );
}

#[test]
fn query_encodes_like_python_quote_plus() {
    let built = url(None, &json!({"term": "a b&c=d*e~f/é"}));

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&term=a+b%26c%3Dd%2Ae~f%2F%C3%A9"
    );
}

#[test]
fn query_encodes_a_percent_sign_that_precedes_a_tilde_escape() {
    let built = url(None, &json!({"term": "%7E~"}));

    assert_eq!(
        built,
        "https://api.deepgram.com/v1/listen?model=nova-3&term=%257E~"
    );
}

#[test]
fn query_encodes_the_model_name() {
    let built = config(None).complete_url(None, "a b/c", &IndexMap::new());

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
    let explicit = explicit.map(SecretString::from);
    let headers = config(configured).validate_environment(explicit.as_ref());

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
    let response = decoded(&json!({
        "transcript": "hello world",
        "paragraphs": {"transcript": "ignored"},
        "words": [
            {"word": "hello", "start": 0.5, "end": 1.5},
            {"word": "world", "start": 2, "end": 3},
        ],
    }));

    assert_eq!(response["text"], "hello world");
    assert_eq!(response["task"], "transcribe");
    assert_eq!(response["language"], "en");
    assert_eq!(response["duration"], 12.5);
    assert_eq!(
        response["words"],
        json!([
            {"word": "hello", "start": 0.5, "end": 1.5},
            {"word": "world", "start": 2, "end": 3},
        ])
    );
}

#[test]
fn a_reply_without_words_has_no_words_entry() {
    let response = decoded(&json!({"transcript": "hi"}));

    assert!(response.get("words").is_none());
}

#[test]
fn an_empty_words_list_is_kept_and_uses_the_transcript() {
    let response = decoded(&json!({"transcript": "hi", "words": []}));

    assert_eq!(response["text"], "hi");
    assert_eq!(response["words"], json!([]));
}

#[test]
fn a_null_transcript_leaves_the_text_out() {
    let response = decoded(&json!({"transcript": null}));

    assert!(response.get("text").is_none());
}

#[test]
fn a_diarized_reply_prefers_the_paragraphs_transcript() {
    let response = decoded(&json!({
        "transcript": "plain",
        "paragraphs": {"transcript": "\nSpeaker 0: from paragraphs\n"},
        "words": [word("hello", Some(0)), word("there", Some(1))],
    }));

    assert_eq!(response["text"], "\nSpeaker 0: from paragraphs\n");
}

#[test]
fn a_diarized_reply_without_paragraphs_rebuilds_the_runs() {
    let response = decoded(&json!({
        "transcript": "plain",
        "words": [
            {"word": "hi", "punctuated_word": "Hi,", "start": 0, "end": 1, "speaker": 0},
            word("there", Some(0)),
            word("hello", Some(1)),
            word("back", Some(0)),
        ],
    }));

    assert_eq!(
        response["text"],
        "Speaker 0: Hi, there\nSpeaker 1: hello\n\nSpeaker 0: back\n"
    );
}

#[test]
fn the_speaker_is_stripped_from_the_public_words() {
    let response = decoded(&json!({
        "transcript": "x",
        "words": [{"word": "hi", "start": 1, "end": 2, "speaker": 3, "confidence": 0.9}],
    }));

    assert_eq!(
        response["words"],
        json!([{"word": "hi", "start": 1, "end": 2}])
    );
}

#[test]
fn a_null_speaker_on_the_first_word_turns_diarization_on() {
    let response = decoded(&json!({
        "transcript": "plain",
        "words": [
            {"word": "hi", "start": 1, "end": 2, "speaker": null},
            {"word": "you", "start": 2, "end": 3},
        ],
    }));

    assert_eq!(response["text"], "\nSpeaker None: hi you\n");
}

#[test]
fn a_speaker_on_the_second_word_alone_does_not_turn_diarization_on() {
    let response = decoded(&json!({
        "transcript": "plain",
        "words": [
            {"word": "hi", "start": 1, "end": 2},
            {"word": "you", "start": 2, "end": 3, "speaker": 1},
        ],
    }));

    assert_eq!(response["text"], "plain");
}

#[test]
fn language_is_the_detected_one_unless_it_is_empty_or_null() {
    let channel = |detected: Value| {
        let body = json!({
            "metadata": {"duration": 1},
            "results": {"channels": [{"detected_language": detected, "alternatives": [{"transcript": "t"}]}]},
        });
        let response = decode_deepgram_transcript(&serde_json::to_vec(&body).expect("body"))
            .expect("the reply decodes");
        response.language
    };

    assert_eq!(channel(json!("de")), "de");
    assert_eq!(channel(json!("")), "en");
    assert_eq!(channel(Value::Null), "en");
}

#[test]
fn only_the_first_channel_and_alternative_are_read() {
    let body = json!({
        "metadata": {"duration": 1},
        "results": {"channels": [
            {"alternatives": [{"transcript": "first"}, 7]},
            "not a channel",
        ]},
    });
    let response = decode_deepgram_transcript(&serde_json::to_vec(&body).expect("body"))
        .expect("the reply decodes");

    assert_eq!(response.text.as_deref(), Some("first"));
}

#[test]
fn the_hidden_params_hold_the_complete_provider_payload() {
    let body = json!({
        "metadata": {"duration": 1, "request_id": "r"},
        "results": {"channels": [{"alternatives": [{"transcript": "t", "confidence": 0.5}]}]},
        "extra": [1, 2],
    });
    let response = decode_deepgram_transcript(&serde_json::to_vec(&body).expect("body"))
        .expect("the reply decodes");

    assert_eq!(Value::Object(response.hidden_params), body);
}

#[rstest]
#[case::not_an_object(json!([1]))]
#[case::no_results(json!({"metadata": {"duration": 1}}))]
#[case::no_metadata(json!({"results": {"channels": [{"alternatives": [{"transcript": "t"}]}]}}))]
#[case::no_duration(json!({"metadata": {}, "results": {"channels": [{"alternatives": [{"transcript": "t"}]}]}}))]
#[case::null_duration(json!({"metadata": {"duration": null}, "results": {"channels": [{"alternatives": [{"transcript": "t"}]}]}}))]
#[case::no_channels(json!({"metadata": {"duration": 1}, "results": {}}))]
#[case::empty_channels(json!({"metadata": {"duration": 1}, "results": {"channels": []}}))]
#[case::null_channels(json!({"metadata": {"duration": 1}, "results": {"channels": null}}))]
#[case::no_alternatives(json!({"metadata": {"duration": 1}, "results": {"channels": [{}]}}))]
#[case::empty_alternatives(json!({"metadata": {"duration": 1}, "results": {"channels": [{"alternatives": []}]}}))]
#[case::no_transcript(json!({"metadata": {"duration": 1}, "results": {"channels": [{"alternatives": [{}]}]}}))]
fn a_reply_without_the_required_members_is_rejected(#[case] body: Value) {
    let error = decode_deepgram_transcript(&serde_json::to_vec(&body).expect("body"))
        .expect_err("the shape is incomplete");

    assert!(matches!(
        error,
        DeepgramTranscriptionError::Transform { .. }
    ));
}

#[rstest]
#[case::null_words(json!({"transcript": "t", "words": null}))]
#[case::word_without_text(json!({"transcript": "t", "words": [{"start": 1, "end": 2}]}))]
#[case::word_without_start(json!({"transcript": "t", "words": [{"word": "a", "end": 2}]}))]
#[case::word_without_end(json!({"transcript": "t", "words": [{"word": "a", "start": 2}]}))]
#[case::word_not_an_object(json!({"transcript": "t", "words": ["a"]}))]
#[case::words_not_a_list(json!({"transcript": "t", "words": {"word": "a"}}))]
#[case::diarized_null_paragraphs(json!({"transcript": "t", "paragraphs": null, "words": [word("a", Some(0))]}))]
#[case::diarized_paragraphs_without_transcript(json!({"transcript": "t", "paragraphs": {}, "words": [word("a", Some(0))]}))]
#[case::diarized_null_punctuated_word(json!({"words": [{"word": "a", "punctuated_word": null, "start": 1, "end": 2, "speaker": 0}]}))]
fn a_malformed_alternative_is_rejected(#[case] alternative: Value) {
    let error =
        decode_deepgram_transcript(&reply(&alternative)).expect_err("the alternative is malformed");

    assert!(matches!(
        error,
        DeepgramTranscriptionError::Transform { .. }
    ));
}

#[test]
fn null_paragraphs_are_ignored_when_the_reply_is_not_diarized() {
    let response = decoded(&json!({"transcript": "plain", "paragraphs": null}));

    assert_eq!(response["text"], "plain");
}

#[test]
fn a_diarized_reply_does_not_need_a_transcript() {
    let response = decoded(&json!({"words": [word("a", Some(0))]}));

    assert_eq!(response["text"], "\nSpeaker 0: a\n");
}

#[test]
fn a_failure_carries_the_raw_body_text() {
    let error = decode_deepgram_transcript(b"not json {").expect_err("the body is not JSON");

    let message = error.to_string();
    assert!(message.starts_with("Error transforming Deepgram response: "));
    assert!(message.ends_with("\nResponse: not json {"));
}

#[test]
fn bad_utf8_bytes_fail_and_show_replacement_characters() {
    let error =
        decode_deepgram_transcript(&[0xff, 0xfe, b'{']).expect_err("the bytes are not UTF-8");

    assert!(error.to_string().ends_with("\nResponse: \u{fffd}\u{fffd}{"));
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

fn spoken(text: &str, speaker: Option<i64>) -> Word {
    serde_json::from_value(word(text, speaker)).expect("a word")
}

#[test]
fn reconstruction_of_no_words_is_empty() {
    assert_eq!(reconstruct_diarized_transcript(&[]).expect("no words"), "");
}

#[test]
fn reconstruction_of_one_run_has_a_leading_and_trailing_newline() {
    let text = reconstruct_diarized_transcript(&[spoken("a", Some(2)), spoken("b", Some(2))])
        .expect("one run");

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
        reconstruct_diarized_transcript(&words).expect("four runs"),
        "Speaker 0: a\nSpeaker 1: b\nSpeaker 0: c\n\nSpeaker 1: d\n"
    );
}

#[test]
fn reconstruction_prefers_the_punctuated_word_and_falls_back_to_the_word() {
    let punctuated: Word = serde_json::from_value(json!(
        {"word": "hi", "punctuated_word": "Hi!", "start": 0, "end": 1, "speaker": 0}
    ))
    .expect("a word");

    assert_eq!(
        reconstruct_diarized_transcript(&[punctuated, spoken("you", Some(0))]).expect("a run"),
        "\nSpeaker 0: Hi! you\n"
    );
}

#[test]
fn reconstruction_treats_a_missing_speaker_like_a_null_one() {
    let bare: Word =
        serde_json::from_value(json!({"word": "b", "start": 0, "end": 1})).expect("a word");

    assert_eq!(
        reconstruct_diarized_transcript(&[spoken("a", Some(1)), spoken("n", None), bare])
            .expect("two runs"),
        "Speaker 1: a\n\nSpeaker None: n b\n"
    );
}

fn corpus_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn expected_corpus_text(name: &str) -> &'static str {
    match name {
        "ok-diarized-paragraphs.json" => "\nSpeaker 0: Hello there.\n",
        "ok-diarized-words.json" => "Speaker 0: Hi,\nSpeaker 1: you\n\nSpeaker 0: ok\n",
        "ok-null-speaker.json" => "\nSpeaker None: x\n",
        "ok-plain.json" => "hello there",
        "ok-speaker-on-second-word.json" => "x",
        other => panic!("the seed {other} has no expected transcript"),
    }
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
    let transcript = outcome.expect("an ok seed decodes");
    assert_eq!(
        transcript.text.as_deref(),
        Some(expected_corpus_text(&name))
    );
}
