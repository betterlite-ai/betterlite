#![cfg(test)]

use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use super::DEFAULT_MUSE_REALTIME_URL;
use super::MUSE_MODEL;
use super::MuseMode;
use super::MuseProtocolError;
use super::MuseSampleRate;
use super::build_muse_realtime_url;
use super::get_complete_url;
use super::normalize_access_token;
use super::normalize_language;
use super::normalize_model;
use super::parse_session_update;
use crate::api::api_boundaries::ApiLimits;
use crate::meta_muse_realtime::support::Capture;
use crate::meta_muse_realtime::support::capture;
use crate::meta_muse_realtime::support::captured_messages;

const BETA_SESSION: &str = r#"{"type":"session.update","session":{"type":"transcription","input_audio_format":"pcm16","input_audio_transcription":{"model":"muse-voice-transcribe-1.0","language":"es"}}}"#;
const GA_SESSION: &str = r#"{"type":"session.update","session":{"type":"transcription","audio":{"input":{"format":{"type":"audio/pcm","rate":16000,"channels":1},"transcription":{"model":"muse-voice-transcribe-1.0","language":"pt-BR"},"turn_detection":{"type":"server_vad"}}}}}"#;
const PUSH_TO_TALK_SESSION: &str = r#"{"type":"session.update","session":{"type":"realtime","audio":{"input":{"transcription":{}}},"turn_detection":null}}"#;
const MINIMAL_SESSION: &str =
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{}}}}}"#;

fn configured(payload: &str) -> Result<super::MuseSessionConfig, MuseProtocolError> {
    parse_session_update(payload, MUSE_MODEL, ApiLimits::default())
}

#[test]
fn strips_the_meta_prefix_and_whitespace_from_models() {
    assert_eq!(
        normalize_model("meta/muse-voice-transcribe-1.0"),
        MUSE_MODEL
    );
    assert_eq!(normalize_model("  muse-voice-transcribe-1.0  "), MUSE_MODEL);
    assert_eq!(normalize_model("other/model"), "other/model");
    assert_eq!(normalize_model("meta/"), "");
}

#[test]
fn normalizes_supported_language_names_and_codes() {
    let cases = [
        (" Spanish ", "Spanish"),
        ("MANDARIN CHINESE", "Mandarin Chinese"),
        ("pt-BR", "Portuguese"),
        ("zh_CN", "Mandarin Chinese"),
        ("fil", "Tagalog"),
        ("iw", "Hebrew"),
        ("en-US-x", "English"),
    ];
    let normalized: Vec<String> = cases
        .iter()
        .map(|(language, _)| normalize_language(language).unwrap())
        .collect();
    let expected: Vec<String> = cases.iter().map(|(_, name)| (*name).to_owned()).collect();
    assert_eq!(normalized, expected);
}

#[test]
fn rejects_empty_and_unsupported_languages() {
    let empty = normalize_language("   ").unwrap_err();
    assert_eq!(empty.to_string(), "language must be non-empty");
    let unsupported = normalize_language("klingon").unwrap_err();
    assert_eq!(unsupported.to_string(), "unsupported Muse Voice language");
}

#[test]
fn normalizes_access_tokens() {
    let cases = [
        ("abc", "Bearer abc"),
        ("  abc  ", "Bearer abc"),
        ("Bearer abc", "Bearer abc"),
        ("bearer   abc  ", "Bearer abc"),
        ("BEARER abc", "Bearer abc"),
    ];
    let normalized: Vec<String> = cases
        .iter()
        .map(|(token, _)| {
            normalize_access_token(&SecretString::from((*token).to_owned()))
                .unwrap()
                .expose_secret()
                .to_owned()
        })
        .collect();
    let expected: Vec<String> = cases
        .iter()
        .map(|(_, bearer)| (*bearer).to_owned())
        .collect();
    assert_eq!(normalized, expected);
}

#[test]
fn rejects_missing_and_blank_tokens() {
    let missing = normalize_access_token(&SecretString::from(String::new())).unwrap_err();
    assert_eq!(missing.to_string(), "Meta API key is required");
    let blank = normalize_access_token(&SecretString::from("   ".to_owned())).unwrap_err();
    assert!(matches!(blank, MuseProtocolError::ApiKeyRequired));
    let bare = normalize_access_token(&SecretString::from("Bearer".to_owned())).unwrap_err();
    assert_eq!(
        bare.to_string(),
        "Meta API key must include a token after Bearer"
    );
    let padded = normalize_access_token(&SecretString::from("bearer    ".to_owned())).unwrap_err();
    assert!(matches!(padded, MuseProtocolError::BearerTokenMissing));
}

#[test]
fn builds_the_default_and_custom_realtime_urls() {
    assert_eq!(
        build_muse_realtime_url(None).unwrap(),
        DEFAULT_MUSE_REALTIME_URL
    );
    let cases = [
        (
            "https://api.example.com/base?x=1",
            "wss://api.example.com/v1/asr/realtime",
        ),
        (
            "  wss://example.com:8443/x  ",
            "wss://example.com:8443/v1/asr/realtime",
        ),
        (
            "https://example.com:9443",
            "wss://example.com:9443/v1/asr/realtime",
        ),
        // LiteLLM drops the IPv6 brackets, so the URL it rebuilds here cannot be parsed
        ("wss://[::1]:9443/x", "wss://[::1]:9443/v1/asr/realtime"),
        // the url crate punycodes a non-ASCII host, while LiteLLM keeps the text urllib.parse reports
        (
            "wss://bücher.example/x",
            "wss://xn--bcher-kva.example/v1/asr/realtime",
        ),
        // the url crate drops a port that is the scheme default, so the rebuilt URL keeps the endpoint without it
        (
            "wss://example.com:443/x",
            "wss://example.com/v1/asr/realtime",
        ),
    ];
    let built: Vec<String> = cases
        .iter()
        .map(|(base, _)| build_muse_realtime_url(Some(base)).unwrap())
        .collect();
    let expected: Vec<String> = cases.iter().map(|(_, url)| (*url).to_owned()).collect();
    assert_eq!(built, expected);
}

#[test]
fn rejects_unusable_api_bases() {
    let bases = [
        "http://api.example.com",
        "api.example.com",
        "https://user:secret@api.example.com",
        "https://api.example.com/#fragment",
        "mailto:someone@example.com",
    ];
    let rejected: Vec<bool> = bases
        .iter()
        .map(|base| {
            matches!(
                build_muse_realtime_url(Some(base)),
                Err(MuseProtocolError::InvalidApiBase)
            )
        })
        .collect();
    assert!(rejected.iter().all(|rejected_base| *rejected_base));
}

#[test]
fn get_complete_url_rejects_models_other_than_muse() {
    assert_eq!(
        get_complete_url(None, "meta/muse-voice-transcribe-1.0 ").unwrap(),
        DEFAULT_MUSE_REALTIME_URL
    );
    let rejected = get_complete_url(None, "gpt-4o-realtime").unwrap_err();
    assert_eq!(
        rejected.to_string(),
        "Unsupported Meta realtime model: gpt-4o-realtime"
    );
}

#[test]
fn parses_a_beta_transcription_session() {
    let config = configured(BETA_SESSION).unwrap();
    assert_eq!(config.model, MUSE_MODEL);
    assert_eq!(config.mode, MuseMode::Endpointing);
    assert_eq!(config.sample_rate, MuseSampleRate::Hz24000);
    assert_eq!(config.language_bias, vec!["Spanish".to_owned()]);
}

#[test]
fn parses_a_ga_transcription_session() {
    let config = configured(GA_SESSION).unwrap();
    assert_eq!(config.mode, MuseMode::Endpointing);
    assert_eq!(config.sample_rate, MuseSampleRate::Hz16000);
    assert_eq!(config.language_bias, vec!["Portuguese".to_owned()]);
}

#[test]
fn parses_ga_sessions_with_a_default_or_explicit_24000_rate() {
    let absent = r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm"},"transcription":{}}}}}"#;
    assert_eq!(
        configured(absent).unwrap().sample_rate,
        MuseSampleRate::Hz24000
    );
    let explicit = r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{}}}}}"#;
    assert_eq!(
        configured(explicit).unwrap().sample_rate,
        MuseSampleRate::Hz24000
    );
}

#[test]
fn parses_push_to_talk_turn_detection() {
    let config = configured(PUSH_TO_TALK_SESSION).unwrap();
    assert_eq!(config.mode, MuseMode::PushToTalk);
    assert_eq!(config.sample_rate, MuseSampleRate::Hz24000);
    assert!(config.language_bias.is_empty());
}

#[test]
fn rejects_unsupported_session_types() {
    let payload = r#"{"type":"session.update","session":{"type":"conversation","audio":{"input":{"transcription":{}}}}}"#;
    let rejected = configured(payload).unwrap_err();
    assert_eq!(
        rejected.to_string(),
        "Muse Voice supports transcription sessions only"
    );
}

#[test]
fn rejects_a_changed_model() {
    let payload = r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"meta/other-model"}}}}}"#;
    let rejected = configured(payload).unwrap_err();
    assert_eq!(
        rejected.to_string(),
        "realtime session model cannot be changed"
    );
}

#[test]
fn rejects_unsupported_expected_models() {
    let rejected =
        parse_session_update(MINIMAL_SESSION, "gpt-4o-realtime", ApiLimits::default()).unwrap_err();
    assert_eq!(rejected.to_string(), "unsupported Meta realtime model");
}

#[test]
fn rejects_bad_beta_audio_formats() {
    let payload = r#"{"type":"session.update","session":{"input_audio_format":"pcm24","input_audio_transcription":{}}}"#;
    let rejected = configured(payload).unwrap_err();
    assert_eq!(
        rejected.to_string(),
        "Muse Voice requires pcm16 input audio"
    );
}

#[test]
fn rejects_non_pcm_ga_formats() {
    let payload = r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/webm","rate":24000,"channels":1},"transcription":{}}}}}"#;
    let rejected = configured(payload).unwrap_err();
    assert_eq!(
        rejected.to_string(),
        "Muse Voice requires audio/pcm input audio"
    );
}

#[test]
fn rejects_stereo_audio() {
    let payload = r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":24000,"channels":2},"transcription":{}}}}}"#;
    let rejected = configured(payload).unwrap_err();
    assert_eq!(rejected.to_string(), "Muse Voice requires mono input audio");
}

#[test]
fn rejects_unsupported_sample_rates() {
    let payload = r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"pcm16","rate":8000},"transcription":{}}}}}"#;
    let rejected = configured(payload).unwrap_err();
    assert_eq!(
        rejected.to_string(),
        "Muse Voice supports PCM16 at 16000 Hz or 24000 Hz"
    );
}

#[test]
fn rejects_non_server_vad_turn_detection() {
    let named = r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{},"turn_detection":{"type":"semantic_vad"}}}}}"#;
    let rejected = configured(named).unwrap_err();
    assert_eq!(
        rejected.to_string(),
        "Muse Voice supports server_vad turn detection or null"
    );
    let numeric = r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{},"turn_detection":{"type":5}}}}}"#;
    let rejected = configured(numeric).unwrap_err();
    assert!(matches!(
        rejected,
        MuseProtocolError::TurnDetectionUnsupported
    ));
}

fn warnings(captured: &Capture) -> Vec<String> {
    let mut messages = captured_messages(captured);
    messages.retain(|message| message.contains("Meta realtime:"));
    messages
}

#[test]
fn warns_about_unsupported_transcription_keys() {
    let (captured, _guard) = capture();
    let payload = r#"{"type":"session.update","session":{"input_audio_format":"pcm16","input_audio_transcription":{"model":"muse-voice-transcribe-1.0","prompt":"hint","temperature":0}}}"#;
    let config = configured(payload).unwrap();
    assert_eq!(config.sample_rate, MuseSampleRate::Hz24000);
    assert_eq!(
        warnings(&captured),
        vec![
            "Meta realtime: dropping unsupported transcription settings [\"prompt\", \"temperature\"]"
                .to_owned()
        ]
    );
    let quiet = configured(MINIMAL_SESSION).unwrap();
    assert_eq!(quiet.sample_rate, MuseSampleRate::Hz24000);
    assert_eq!(warnings(&captured).len(), 1);
}
