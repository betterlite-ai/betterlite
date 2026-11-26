#![cfg(test)]

use super::{hidden_params, prepare, request_duration};
use crate::audio::convert::{AudioCallWire, AudioSettings, parse};
use crate::audio::session::AudioSession;
use crate::completion::setup::outbound_client;
use betterlite_models::call_helpers::headers::{AdditionalHeaders, ProviderHeaderProjection};
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_files::CallFile;
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::iter::repeat_n;

fn session() -> AudioSession {
    AudioSession {
        environment: BTreeMap::new(),
        client: outbound_client(&BTreeMap::new()).unwrap(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    }
}

fn wav_bytes() -> Vec<u8> {
    let data_len = 16_000_u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&36_u32.saturating_add(data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8000_u32.to_le_bytes());
    wav.extend_from_slice(&16_000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend(repeat_n(0_u8, 16_000));
    wav
}

fn parse_wire(payload: &str) -> (AudioCallWire, Vec<(String, CallFile)>) {
    let mut blobs = vec![wav_bytes()];
    parse(&format!(r#"{{"arguments":{payload}}}"#), &mut blobs).unwrap()
}

fn processed() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: wav_bytes(),
        filename: String::from("clip.wav"),
        content_type: String::from("audio/wav"),
    }
}

fn reply(body: &str) -> TranscriptionReply {
    let response: TranscriptionResponse = serde_json::from_str(body).unwrap();
    TranscriptionReply {
        response,
        original_response: Json::Null,
        provider_headers: ProviderHeaderProjection {
            headers: IndexMap::from([(String::from("x-request-id"), String::from("req-1"))]),
            additional_headers: AdditionalHeaders::from([(
                String::from("llm_provider-x-request-id"),
                Json::Text(String::from("req-1")),
            )]),
        },
        model: Some(String::from("whisper-1")),
        custom_llm_provider: "openai",
        duration: StoredDuration::Absent,
    }
}

#[test]
fn transcription_prepare_accepts_the_openai_provider() {
    let (call, files) = parse_wire(
        r#"{"model":"openai/whisper-1","file":{"$file":{"name":"clip.wav","blob":0}},
            "api_key":"sk-test","custom_llm_provider":"openai"}"#,
    );
    let prepared = prepare(&session(), &call, &files).unwrap();
    assert_eq!(prepared.model, "whisper-1");
    assert_eq!(prepared.provider, "openai");
    assert_eq!(prepared.input, "clip.wav");
    assert_eq!(prepared.connection.max_retries, 2);
    assert!(prepared.options.is_empty());
}

#[test]
fn transcription_prepare_rejects_another_provider() {
    let (call, files) = parse_wire(
        r#"{"model":"vertex_ai/whisper","file":{"$file":{"name":"clip.wav","blob":0}},
            "api_key":"sk-test"}"#,
    );
    let failure = prepare(&session(), &call, &files).err().unwrap();
    assert!(
        failure
            .message
            .contains("transcription is not supported for vertex_ai")
    );
}

#[test]
fn request_request_duration_reads_the_wav_header() {
    let found = request_duration(&wav_bytes()).unwrap();
    assert!(found > Decimal::ZERO);
    assert_eq!(request_duration(b"not audio"), None);
}

#[test]
fn request_hidden_params_carry_the_handler_values() {
    let request = AudioCallWire {
        arguments: RequestMap::new(),
        settings: AudioSettings::default(),
        atranscription: false,
        file_from_kwargs: false,
    };
    let hidden = hidden_params(
        &reply(r#"{"text":"hello","duration":1.5}"#),
        &processed(),
        12.5_f64,
        &request,
        "whisper-1",
    )
    .unwrap();
    assert_eq!(
        hidden.get("model"),
        Some(&Json::Text(String::from("whisper-1")))
    );
    assert_eq!(
        hidden.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert!(matches!(hidden.get("headers"), Some(Json::Object(_))));
    assert!(matches!(
        hidden.get("additional_headers"),
        Some(Json::Object(_))
    ));
    assert!(matches!(hidden.get("_response_ms"), Some(Json::Number(_))));
    assert_eq!(hidden.get("audio_transcription_duration"), None);
}

#[test]
fn request_hidden_params_compute_the_duration() {
    let request = AudioCallWire {
        arguments: RequestMap::new(),
        settings: AudioSettings::default(),
        atranscription: false,
        file_from_kwargs: false,
    };
    let hidden = hidden_params(
        &reply(r#"{"text":"hello"}"#),
        &processed(),
        1.0_f64,
        &request,
        "whisper-1",
    )
    .unwrap();
    assert!(matches!(
        hidden.get("audio_transcription_duration"),
        Some(Json::Number(_))
    ));
}

#[test]
fn request_hidden_params_compute_for_an_async_kwarg_file() {
    let request = AudioCallWire {
        arguments: RequestMap::new(),
        settings: AudioSettings::default(),
        atranscription: true,
        file_from_kwargs: true,
    };
    let hidden = hidden_params(
        &reply(r#"{"text":"hello"}"#),
        &processed(),
        1.0_f64,
        &request,
        "whisper-1",
    )
    .unwrap();
    assert!(matches!(
        hidden.get("audio_transcription_duration"),
        Some(Json::Number(_))
    ));
}

#[test]
fn request_hidden_params_take_the_async_duration() {
    let mut stored = reply(r#"{"text":"hello"}"#);
    stored.duration = StoredDuration::Known(Some(Decimal::new(3, 1)));
    let request = AudioCallWire {
        arguments: RequestMap::new(),
        settings: AudioSettings::default(),
        atranscription: true,
        file_from_kwargs: false,
    };
    let hidden = hidden_params(&stored, &processed(), 1.0_f64, &request, "whisper-1").unwrap();
    assert!(matches!(
        hidden.get("audio_transcription_duration"),
        Some(Json::Number(value)) if value.as_f64() == Some(0.3_f64)
    ));
}

#[test]
fn request_hidden_params_omit_absent_async_duration() {
    let request = AudioCallWire {
        arguments: RequestMap::new(),
        settings: AudioSettings::default(),
        atranscription: true,
        file_from_kwargs: false,
    };
    let hidden = hidden_params(
        &reply(r#"{"text":"hello"}"#),
        &processed(),
        1.0_f64,
        &request,
        "whisper-1",
    )
    .unwrap();
    assert_eq!(hidden.get("audio_transcription_duration"), None);
}
