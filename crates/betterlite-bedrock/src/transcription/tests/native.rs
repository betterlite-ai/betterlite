use betterlite_models::decisions::Json;
use rstest::rstest;

use super::native_auth::native_transcription_auth;
use super::native_reply::native_transcript_reply;
use super::native_url::native_transcription_url;
use super::support::params;
use crate::transcription::request::{SUPPORTED_PARAMS, transcription_body};

const FORMAT_ERROR: Option<&str> = Some("audio.format must be wav, mp3, flac, or ogg");
const MISSING_DATA: Option<&str> = Some("missing field audio.data");

const AUDIO_ROWS: [(&str, Option<&str>); 15] = [
    (r#""x""#, Some("expected object, got string")),
    ("null", Some("expected object, got null")),
    ("true", Some("expected object, got boolean")),
    ("1", Some("expected object, got number")),
    ("[]", Some("expected object, got array")),
    (r#"{"format":"wav"}"#, MISSING_DATA),
    (r#"{"data":"","format":"wav"}"#, MISSING_DATA),
    (r#"{"data":1,"format":"wav"}"#, MISSING_DATA),
    (r#"{"data":"QQ=="}"#, FORMAT_ERROR),
    (r#"{"data":"QQ==","format":"WAV"}"#, FORMAT_ERROR),
    (r#"{"data":"QQ==","format":"aac"}"#, FORMAT_ERROR),
    (r#"{"data":"QQ==","format":7}"#, FORMAT_ERROR),
    (r#"{"data":"%%","format":"ogg","filename":"a.aac"}"#, None),
    (r#"{"data":"QQ==","format":"flac"}"#, None),
    (r#"{"data":"QQ==","format":"mp3"}"#, None),
];

const INSTRUCTION: &str = "Transcribe the audio. Respond with only the transcript.";

const REQUEST_ROWS: [(&str, &str, &str); 10] = [
    ("{}", "", ""),
    (r#"{"language":"en"}"#, " The audio language is en.", ""),
    (r#"{"prompt":"names"}"#, " Additional context: names", ""),
    (
        r#"{"prompt":"p","language":"de"}"#,
        " The audio language is de. Additional context: p",
        "",
    ),
    (r#"{"language":"","prompt":""}"#, "", ""),
    (r#"{"language":5,"prompt":[]}"#, "", ""),
    (r#"{"response_format":"json"}"#, "", ""),
    (r#"{"temperature":0}"#, "", r#","temperature":0"#),
    (r#"{"temperature":null}"#, "", r#","temperature":null"#),
    (r#"{"temperature":0.5}"#, "", r#","temperature":0.5"#),
];

fn native_audio_validation() {
    for (source, expected) in AUDIO_ROWS {
        let audio: Json = serde_json::from_str(source).unwrap();
        let failure = transcription_body(&audio, &params("{}"))
            .err()
            .map(|error| error.to_string());
        assert_eq!(failure.as_deref(), expected, "{source}");
    }
}

fn native_transcription_request() {
    assert_eq!(
        SUPPORTED_PARAMS,
        ["language", "prompt", "temperature", "response_format"]
    );
    let audio: Json = serde_json::from_str(r#"{"data":"QQ==","format":"wav"}"#).unwrap();
    for (source, suffix, temperature) in REQUEST_ROWS {
        let expected = format!(
            r#"{{"messages":[{{"role":"user","content":[{{"audio":{{"format":"wav","source":{{"bytes":"QQ=="}}}}}},{{"text":"{INSTRUCTION}{suffix}"}}]}}],"system":[{{"text":"You are a transcription assistant."}}],"inferenceConfig":{{"maxTokens":4096{temperature}}}}}"#
        );
        let body = transcription_body(&audio, &params(source)).unwrap();
        assert_eq!(body, expected, "{source}");
    }
}

#[rstest]
#[case::native_audio_validation(native_audio_validation)]
#[case::native_transcription_request(native_transcription_request)]
#[case::native_transcription_url(native_transcription_url)]
#[case::native_transcription_auth(native_transcription_auth)]
#[case::native_transcript_reply(native_transcript_reply)]
fn transcription_native_cases(#[case] check: fn()) {
    check();
}
