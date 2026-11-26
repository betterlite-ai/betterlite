#![cfg(test)]

use super::speech::{speech_timeout, speech_values};
use super::{
    AudioCallWire, ConvertError, NoFiles, SpeechCallWire, UNNAMED_FILE, audio_file,
    bare_string_file, family_for, first_file, max_retries, parse, parse_speech, resolve_target,
    timeout, transcription_options,
};
use crate::audio::error::provider_failure;
use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles as _;
use betterlite_openai::files::error::StatusError;
use betterlite_openai::speech::request::SpeechFormat;
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_files::{CallFile, CallFilesError};
use betterlite_sdk::call_setup::RequestMap;
use rust_decimal::Decimal;
use std::io;
use std::path::Path;
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
fn request_target_takes_the_model_prefix_or_the_argument() {
    let (model, provider) = resolve_target("openai/whisper-1", None);
    assert_eq!(model, "whisper-1");
    assert_eq!(provider, "openai");
    let (matched, kept) = resolve_target("openai/whisper-1", Some("openai"));
    assert_eq!(matched, "whisper-1");
    assert_eq!(kept, "openai");
    let (whole, given) = resolve_target("openai/whisper-1", Some("azure"));
    assert_eq!(whole, "openai/whisper-1");
    assert_eq!(given, "azure");
    let (plain, defaulted) = resolve_target("whisper-1", None);
    assert_eq!(plain, "whisper-1");
    assert_eq!(defaulted, "openai");
    let (bare, named) = resolve_target("whisper-1", Some("openai"));
    assert_eq!(bare, "whisper-1");
    assert_eq!(named, "openai");
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
    bare_string_file(&call.arguments).unwrap();
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
    let error = speech_values(&call.arguments).err().unwrap();
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

#[test]
fn request_first_file_takes_the_file_entry() {
    let files = [
        (String::from("image"), file("other.wav", None)),
        (String::from("file"), file("clip.wav", None)),
    ];
    assert_eq!(first_file(&files).unwrap().name, "clip.wav");
    let other = [(String::from("image"), file("other.wav", None))];
    assert!(first_file(&other).is_none());
}

#[test]
fn request_max_retries_reads_the_argument() {
    let call = wire(r#"{"max_retries":3}"#);
    assert_eq!(max_retries(&call.arguments).unwrap(), Some(3));
    let absent = RequestMap::new();
    assert_eq!(max_retries(&absent).unwrap(), None);
    let negative = wire(r#"{"max_retries":-4}"#);
    assert_eq!(max_retries(&negative.arguments).unwrap(), Some(0));
    let wrong = wire(r#"{"max_retries":"3"}"#);
    assert!(matches!(
        max_retries(&wrong.arguments),
        Err(ConvertError::Field { .. })
    ));
}

#[test]
fn error_no_files_never_reads_a_path() {
    let error = NoFiles.read(Path::new("clip.wav")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(matches!(
        NoFiles.open(Path::new("clip.wav")),
        Err(open_error) if open_error.kind() == io::ErrorKind::NotFound
    ));
}

#[test]
fn request_speech_timeout_reads_the_argument() {
    let call = speech_wire(r#"{"model":"tts-1","input":"hi","timeout":2.5}"#);
    assert_eq!(
        speech_timeout(&call.arguments).unwrap(),
        Some(Duration::from_millis(2500))
    );
    let absent = RequestMap::new();
    assert_eq!(speech_timeout(&absent).unwrap(), None);
}

#[test]
fn error_provider_body_keeps_fields() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    for status in [400_u16, 401_u16, 500_u16] {
        let error = StatusError::from_response(
            status,
            br#"{"error":{"type":"invalid_request_error","param":"file","code":"invalid_file"}}"#,
        );
        let failure = provider_failure(
            error.message.clone(),
            Some(&error),
            "whisper-1",
            "openai",
            &redactor,
        );
        assert_eq!(failure.status_code, Some(status));
        let body = failure.body.unwrap();
        assert_eq!(body.kind.as_deref(), Some("invalid_request_error"));
        assert_eq!(body.param.as_deref(), Some("file"));
        assert_eq!(body.code.as_deref(), Some("invalid_file"));
    }
}

#[test]
fn error_transport_body_is_absent() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let failure = provider_failure(
        "connection refused".to_owned(),
        None,
        "whisper-1",
        "openai",
        &redactor,
    );
    assert!(failure.body.is_none());
    assert_eq!(failure.kind, "APIConnectionError");
}
