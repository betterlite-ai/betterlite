#![cfg(test)]

use super::speech::speech_values;
use super::{
    AudioCallWire, ConvertError, SpeechCallWire, UNNAMED_FILE, audio_file, bare_string_file,
    family_for, parse, parse_speech, timeout, transcription_options,
};
use betterlite_openai::speech::request::SpeechFormat;
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_sdk::call_files::{CallFile, CallFilesError};
use rust_decimal::Decimal;
use std::str::FromStr as _;
use std::time::Duration;

fn wire(text: &str) -> AudioCallWire {
    parse(&format!(r#"{{"arguments":{text}}}"#), &mut [])
        .unwrap()
        .0
}

fn speech_wire(text: &str) -> SpeechCallWire {
    parse_speech(&format!(r#"{{"arguments":{text}}}"#)).unwrap()
}

fn file(name: &str, content_type: Option<&str>) -> CallFile {
    CallFile {
        name: name.to_owned(),
        content_type: content_type.map(str::to_owned),
        bytes: b"RIFF....WAVEfmt ".to_vec(),
    }
}

#[test]
fn request_options_take_the_five_named_params() {
    let call = wire(
        r#"{"model":"whisper-1","language":"en","prompt":"hi","response_format":null,
            "temperature":0,"timestamp_granularities":["word"],"user":"u"}"#,
    );
    let options = transcription_options(&call.arguments);
    let names: Vec<&str> = options.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "language",
            "prompt",
            "temperature",
            "timestamp_granularities"
        ]
    );
}

#[test]
fn request_family_for_a_gpt4o_model() {
    assert_eq!(family_for("gpt-4o-transcribe"), TranscriptionFamily::Gpt);
    assert_eq!(
        family_for("gpt-4o-mini-transcribe"),
        TranscriptionFamily::Gpt
    );
    assert_eq!(family_for("whisper-1"), TranscriptionFamily::Whisper);
}

#[test]
fn request_file_without_a_name_becomes_audio_wav() {
    let unnamed = file(UNNAMED_FILE, None);
    let (processed, input) = audio_file(&unnamed).unwrap();
    assert_eq!(processed.filename, "audio.wav");
    assert_eq!(processed.content_type, "audio/wav");
    assert_eq!(input, "audio.wav");
}

#[test]
fn request_file_extension_sets_the_type() {
    let named = file("clip.wav", None);
    let (processed, input) = audio_file(&named).unwrap();
    assert_eq!(processed.filename, "clip.wav");
    assert_eq!(processed.content_type, "audio/wav");
    assert_eq!(input, "clip.wav");
}

#[test]
fn request_tuple_type_wins() {
    let marked = file("clip.wav", Some("audio/x-custom"));
    let (processed, _input) = audio_file(&marked).unwrap();
    assert_eq!(processed.content_type, "audio/x-custom");
}

#[test]
fn request_file_argument_takes_a_marker() {
    let call = wire(r#"{"file":{"$file":{"name":"clip.wav","blob":0}}}"#);
    assert!(bare_string_file(&call.arguments).is_ok());
}

#[test]
fn request_speech_values_map_the_optional_params() {
    let call = speech_wire(
        r#"{"model":"tts-1","input":"hello","voice":"alloy",
            "response_format":"mp3","speed":1.5,"instructions":"calm"}"#,
    );
    let values = speech_values(&call.arguments).unwrap();
    assert_eq!(values.model, "tts-1");
    assert_eq!(values.input, "hello");
    assert_eq!(values.voice.as_deref(), Some("alloy"));
    assert_eq!(values.options.response_format, Some(SpeechFormat::Mp3));
    assert_eq!(
        values.options.speed,
        Some(Decimal::from_str("1.5").unwrap())
    );
    assert_eq!(values.options.instructions.as_deref(), Some("calm"));
}

#[test]
fn request_speech_empty_voice_is_kept() {
    let call = speech_wire(r#"{"model":"tts-1","input":"hello","voice":""}"#);
    let values = speech_values(&call.arguments).unwrap();
    assert_eq!(values.voice.as_deref(), Some(""));
}

#[test]
fn request_timeout_defaults_to_ten_minutes() {
    let call = wire("{}");
    assert_eq!(timeout(&call.arguments).unwrap(), Duration::from_secs(600));
}

#[test]
fn request_timeout_reads_the_argument() {
    let call = wire(r#"{"timeout":2.5}"#);
    assert_eq!(
        timeout(&call.arguments).unwrap(),
        Duration::from_millis(2500)
    );
}

#[test]
fn error_payload_must_be_json() {
    let mut blobs = Vec::new();
    assert!(matches!(
        parse("not json", &mut blobs),
        Err(ConvertError::Json { .. })
    ));
}

#[test]
fn error_speech_payload_must_be_json() {
    assert!(matches!(
        parse_speech("not json"),
        Err(ConvertError::Json { .. })
    ));
}

#[test]
fn error_bare_string_file() {
    let call = wire(r#"{"file":"clip.wav"}"#);
    let error = bare_string_file(&call.arguments).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not accept bare str inputs")
    );
}

#[test]
fn error_file_marker_without_a_blob() {
    let mut blobs = Vec::new();
    let error = parse(r#"{"arguments":{"file":{"$file":{}}}}"#, &mut blobs).unwrap_err();
    assert!(matches!(
        error,
        ConvertError::Files(CallFilesError::MissingBlob { .. })
    ));
}

#[test]
fn error_unknown_response_format() {
    let call = speech_wire(r#"{"model":"tts-1","input":"hi","response_format":"ogg"}"#);
    let error = speech_values(&call.arguments).unwrap_err();
    assert!(matches!(error, ConvertError::Field { name, .. } if name == "response_format"));
}

#[test]
fn error_timeout_must_be_a_number() {
    let call = wire(r#"{"timeout":"soon"}"#);
    assert!(matches!(
        timeout(&call.arguments),
        Err(ConvertError::Field { .. })
    ));
}
