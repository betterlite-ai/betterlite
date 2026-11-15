#![cfg(test)]

mod logging;
mod words;

use std::str::FromStr as _;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use rstest::rstest;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::api::api_boundaries::ApiLimits;
use crate::api::transcription_protocol::events::{
    AudioFormatType, EventIdSource, SERVER_VAD_TURN_DETECTION, SessionCreatedKind, SessionFormat,
    TranscriptionSettings,
};
use crate::api::transcription_protocol::usage::duration_usage;

use super::events::{
    ChirpBackendEvent, ChirpEvent, ChirpSpeechEvent, ChirpStreamingResponse, ChirpStreamingResult,
    ChirpTurnDiscarded,
};
use super::transformer::{ChirpEventTransformer, ItemIdSource, UuidItemIds};
use super::{
    ChirpProtocolError, ChirpSessionConfig, DEFAULT_SAMPLE_RATE_HERTZ, DISCARD_TURN_COMMAND,
    FINISH_TURN_COMMAND, MAX_AUDIO_MESSAGE_BYTES, MAX_SAMPLE_RATE_HERTZ, MIN_SAMPLE_RATE_HERTZ,
    ProtocolError, created_event, default_session_config, is_vertex_speech_to_text_model,
    normalize_speech_to_text_model, parse_chirp_session_update,
};

const STARTED: &str = "input_audio_buffer.speech_started";
const STOPPED: &str = "input_audio_buffer.speech_stopped";
const DELTA: &str = "conversation.item.input_audio_transcription.delta";
const COMPLETED: &str = "conversation.item.input_audio_transcription.completed";

fn parse(payload: &str) -> Result<ChirpSessionConfig, ChirpProtocolError> {
    parse_chirp_session_update(payload.as_bytes(), "chirp_2", ApiLimits::default())
}

fn session_config(server_vad: bool) -> ChirpSessionConfig {
    ChirpSessionConfig {
        model: "chirp_2".to_owned(),
        language: Some("en-US".to_owned()),
        sample_rate: 16_000,
        server_vad,
    }
}

#[derive(Default)]
struct FixedEventIds {
    next: usize,
}

impl EventIdSource for FixedEventIds {
    fn next_event_id(&mut self) -> String {
        let identifier = format!("event_{}", self.next);
        self.next = self.next.saturating_add(1);
        identifier
    }
}

#[derive(Default)]
struct FixedItemIds {
    next: usize,
}

impl ItemIdSource for FixedItemIds {
    fn next_item_id(&mut self) -> String {
        let identifier = format!("item_{}", self.next);
        self.next = self.next.saturating_add(1);
        identifier
    }
}

fn chirp_transformer(server_vad: bool) -> ChirpEventTransformer<FixedEventIds, FixedItemIds> {
    let mut transformer =
        ChirpEventTransformer::new(FixedEventIds::default(), FixedItemIds::default());
    transformer.configure(session_config(server_vad), "sess_1");
    transformer
}

fn frame(
    speech_event: ChirpSpeechEvent,
    results: &[(&str, bool)],
    billed_seconds: &str,
) -> ChirpBackendEvent {
    ChirpBackendEvent::Response(ChirpStreamingResponse {
        speech_event,
        results: results
            .iter()
            .map(|(transcript, is_final)| ChirpStreamingResult {
                transcript: (*transcript).to_owned(),
                is_final: *is_final,
            })
            .collect(),
        billed_seconds: Decimal::from_str(billed_seconds).unwrap(),
    })
}

#[derive(Deserialize)]
struct WireEvent {
    #[serde(rename = "type")]
    event_type: String,
    event_id: String,
    item_id: Option<String>,
    delta: Option<String>,
    transcript: Option<String>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(rename = "type")]
    usage_type: String,
    #[serde(with = "rust_decimal::serde::float")]
    seconds: Decimal,
}

impl WireEvent {
    fn reported_usage(&self) -> &WireUsage {
        self.usage.as_ref().expect("a usage report")
    }
}

fn wire_events(events: &[ChirpEvent]) -> Vec<WireEvent> {
    events
        .iter()
        .map(|event| {
            let wire = serde_json::to_string(event).expect("a serializable event");
            serde_json::from_str(&wire).expect("a wire event")
        })
        .collect()
}

fn event_types(documents: &[WireEvent]) -> Vec<&str> {
    documents
        .iter()
        .map(|document| document.event_type.as_str())
        .collect()
}

#[rstest]
#[case::chirp_2("chirp_2", true)]
#[case::chirp_3("chirp_3", true)]
#[case::prefixed("vertex_ai/chirp_2", true)]
#[case::chirp_speech("chirp", false)]
#[case::gemini_preview("gemini-3.5-transcribe-preview", false)]
#[case::gemini_live_preview("gemini-3.5-transcribe-live-preview", false)]
#[case::gemini_without_endpoints("gemini-3.5-transcribe", false)]
#[case::gemini_live_without_endpoints("gemini-3.5-transcribe-live", false)]
#[case::other_provider("openai/gpt-4o-transcribe", false)]
#[case::unmapped("nowhere-model", false)]
fn recognizes_only_audio_transcription_models_with_both_endpoints(
    #[case] model: &str,
    #[case] expected: bool,
) {
    let catalog = BundledCatalog::default().get();
    assert_eq!(is_vertex_speech_to_text_model(&catalog, model), expected);
}

fn custom_catalog(mode: &str, endpoints: &str) -> ModelCatalog {
    let entry = format!(
        r#"{{"vertex_ai/chirp_2": {{"litellm_provider": "vertex_ai", "mode": "{mode}", "supported_endpoints": {endpoints}}}}}"#
    );
    ModelCatalog::parse(entry.as_bytes(), UnknownFields::Reject).expect("a catalog")
}

#[test]
fn an_entry_without_both_speech_to_text_endpoints_is_not_recognized() {
    let streaming = custom_catalog(
        "audio_transcription",
        r#"["/v1/audio/transcriptions", "/v1/realtime"]"#,
    );
    assert!(is_vertex_speech_to_text_model(&streaming, "chirp_2"));
    let other = custom_catalog("audio_transcription", r#"["/v1/chat/completions"]"#);
    assert!(!is_vertex_speech_to_text_model(&other, "chirp_2"));
    let chat = custom_catalog("chat", r#"["/v1/audio/transcriptions", "/v1/realtime"]"#);
    assert!(!is_vertex_speech_to_text_model(&chat, "chirp_2"));
}

#[rstest]
#[case::prefixed("vertex_ai/chirp_2", "chirp_2")]
#[case::bare("chirp_2", "chirp_2")]
#[case::other_provider("openai/gpt-4o", "openai/gpt-4o")]
#[case::doubled("vertex_ai/vertex_ai/chirp_2", "vertex_ai/chirp_2")]
fn strips_a_single_vertex_model_prefix(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(normalize_speech_to_text_model(model), expected);
}

#[test]
fn default_session_config_is_a_vad_transcription_session() {
    let config = default_session_config("vertex_ai/chirp_3");
    assert_eq!(config.model, "chirp_3");
    assert_eq!(config.language, None);
    assert_eq!(config.sample_rate, DEFAULT_SAMPLE_RATE_HERTZ);
    assert!(config.server_vad);
}

#[test]
fn session_update_defaults_match_the_default_session_config() {
    let parsed = parse(r#"{"type":"session.update","session":{"audio":{}}}"#).unwrap();
    assert_eq!(parsed, default_session_config("chirp_2"));
}

#[test]
fn accepts_the_ga_session_update_event_type() {
    let parsed =
        parse(r#"{"type":"transcription_session.update","session":{"audio":{}}}"#).unwrap();
    assert_eq!(parsed, default_session_config("chirp_2"));
}

#[rstest]
#[case::lowest(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":8000}}}}}"#, 8_000)]
#[case::middle(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":24000}}}}}"#, 24_000)]
#[case::highest(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":48000}}}}}"#, 48_000)]
#[case::beta_pcm16(
    r#"{"type":"session.update","session":{"input_audio_format":"pcm16"}}"#,
    24_000
)]
#[case::without_rate(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm"}}}}}"#,
    24_000
)]
#[case::without_format(r#"{"type":"session.update","session":{"audio":{}}}"#, 24_000)]
fn parses_the_session_sample_rate(#[case] payload: &str, #[case] expected: i64) {
    assert_eq!(parse(payload).unwrap().sample_rate, expected);
}

#[rstest]
#[case::just_below(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":7999}}}}}"#)]
#[case::just_above(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":48001}}}}}"#)]
#[case::zero(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":0}}}}}"#)]
#[case::negative(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":-8000}}}}}"#)]
fn rejects_sample_rates_outside_the_inclusive_range(#[case] payload: &str) {
    let error = parse(payload).unwrap_err();
    assert!(matches!(
        error,
        ChirpProtocolError::SampleRate {
            min: MIN_SAMPLE_RATE_HERTZ,
            max: MAX_SAMPLE_RATE_HERTZ,
        }
    ));
}

#[rstest]
#[case::opus(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/opus"}}}}}"#
)]
#[case::without_encoding(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"rate":16000}}}}}"#
)]
#[case::string_format(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":"custom"}}}}"#
)]
#[case::beta_pcm24(r#"{"type":"session.update","session":{"input_audio_format":"pcm24"}}"#)]
fn rejects_audio_formats_that_are_not_pcm16(#[case] payload: &str) {
    assert!(matches!(
        parse(payload).unwrap_err(),
        ChirpProtocolError::NotPcm16
    ));
}

#[rstest]
#[case::stereo(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","channels":2}}}}}"#)]
#[case::zero(r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","channels":0}}}}}"#)]
fn rejects_channels_other_than_one(#[case] payload: &str) {
    assert!(matches!(
        parse(payload).unwrap_err(),
        ChirpProtocolError::NotMono
    ));
}

#[test]
fn accepts_a_single_channel() {
    let parsed = parse(
        r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":16000,"channels":1}}}}}"#,
    )
    .unwrap();
    assert_eq!(parsed.sample_rate, 16_000);
}

#[rstest]
#[case::absent(r#"{"type":"session.update","session":{"audio":{}}}"#, true)]
#[case::server_vad(r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":{"type":"server_vad"}}}}}"#, true)]
#[case::empty_object(
    r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":{}}}}}"#,
    true
)]
#[case::null_type(
    r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":{"type":null}}}}}"#,
    true
)]
fn parses_server_vad_turn_detection(#[case] payload: &str, #[case] expected: bool) {
    assert_eq!(parse(payload).unwrap().server_vad, expected);
}

#[test]
fn a_null_turn_detection_disables_server_vad() {
    let parsed =
        parse(r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":null}}}}"#)
            .unwrap();
    assert!(!parsed.server_vad);
}

#[rstest]
#[case::semantic(r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":{"type":"semantic_vad"}}}}}"#)]
#[case::number(
    r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":{"type":5}}}}}"#
)]
fn rejects_other_turn_detection_types(#[case] payload: &str) {
    assert!(matches!(
        parse(payload).unwrap_err(),
        ChirpProtocolError::TurnDetection
    ));
}

#[rstest]
#[case::absent(r#"{"type":"session.update","session":{"audio":{}}}"#, true)]
#[case::transcription(
    r#"{"type":"session.update","session":{"type":"transcription","audio":{}}}"#,
    true
)]
#[case::realtime(
    r#"{"type":"session.update","session":{"type":"realtime","audio":{}}}"#,
    true
)]
#[case::translation(
    r#"{"type":"session.update","session":{"type":"translation","audio":{}}}"#,
    false
)]
fn accepts_only_transcription_session_types(#[case] payload: &str, #[case] accepted: bool) {
    let parsed = parse(payload);
    assert_eq!(parsed.is_ok(), accepted);
    if !accepted {
        assert!(matches!(
            parsed.unwrap_err(),
            ChirpProtocolError::SessionType
        ));
    }
}

#[rstest]
#[case::unchanged(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"chirp_2"}}}}}"#,
    "chirp_2",
    true
)]
#[case::prefixed_expected_model(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"chirp_2"}}}}}"#,
    "vertex_ai/chirp_2",
    true
)]
#[case::prefixed_update_model(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"vertex_ai/chirp_2"}}}}}"#,
    "chirp_2",
    true
)]
#[case::changed(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"chirp_3"}}}}}"#,
    "chirp_2",
    false
)]
fn accepts_only_the_requested_model(
    #[case] payload: &str,
    #[case] expected_model: &str,
    #[case] accepted: bool,
) {
    let parsed =
        parse_chirp_session_update(payload.as_bytes(), expected_model, ApiLimits::default());
    assert_eq!(parsed.is_ok(), accepted);
    if !accepted {
        assert!(matches!(
            parsed.unwrap_err(),
            ChirpProtocolError::ModelChanged
        ));
    }
}

#[test]
fn the_configured_model_drops_the_vertex_prefix() {
    let payload = r#"{"type":"session.update","session":{"audio":{}}}"#;
    let parsed = parse_chirp_session_update(
        payload.as_bytes(),
        "vertex_ai/chirp_3",
        ApiLimits::default(),
    )
    .unwrap();
    assert_eq!(parsed.model, "chirp_3");
}

#[rstest]
#[case::bare_iso(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"language":"en"}}}}}"#,
    Some("en-US")
)]
#[case::uppercase_iso(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"language":"EN"}}}}}"#,
    Some("en-US")
)]
#[case::regional(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"language":"fr-CA"}}}}}"#,
    Some("fr-CA")
)]
#[case::unmapped(
    r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"language":"xx"}}}}}"#,
    Some("xx")
)]
#[case::absent(r#"{"type":"session.update","session":{"audio":{}}}"#, None)]
fn normalizes_the_session_language(#[case] payload: &str, #[case] expected: Option<&str>) {
    let parsed = parse(payload).unwrap();
    assert_eq!(parsed.language.as_deref(), expected);
}

#[test]
fn rejects_messages_that_are_not_session_updates() {
    let error = parse(r#"{"type":"session.created"}"#).unwrap_err();
    assert!(matches!(
        error,
        ChirpProtocolError::Protocol(ProtocolError::ExpectedSessionUpdate)
    ));
}

#[test]
fn rejects_invalid_json() {
    let error = parse("{").unwrap_err();
    assert!(matches!(
        error,
        ChirpProtocolError::Protocol(ProtocolError::InvalidJson)
    ));
}

#[test]
fn rejects_a_fractional_sample_rate() {
    let error = parse(
        r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":24000.0}}}}}"#,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ChirpProtocolError::Protocol(ProtocolError::FieldNotInteger { .. })
    ));
}

#[test]
fn session_created_event_describes_the_configured_session() {
    let config = session_config(true);
    let created = created_event("event_0".to_owned(), &config.openai_session("sess_1"));
    assert_eq!(created.kind, SessionCreatedKind::SessionCreated);
    assert_eq!(created.event_id, "event_0");
    assert_eq!(created.session.id, "sess_1");
    assert_eq!(
        created.session.audio.input.format,
        SessionFormat {
            format_type: AudioFormatType::AudioPcm,
            rate: 16_000,
        }
    );
    assert_eq!(
        created.session.audio.input.transcription,
        TranscriptionSettings {
            model: "chirp_2".to_owned(),
            language: Some("en-US".to_owned()),
        }
    );
    assert_eq!(
        created.session.audio.input.turn_detection,
        Some(SERVER_VAD_TURN_DETECTION)
    );
}

#[test]
fn session_created_event_omits_disabled_turn_detection_and_language() {
    let mut config = session_config(false);
    config.language = None;
    let created = created_event("event_0".to_owned(), &config.openai_session("sess_1"));
    assert_eq!(
        created.session.audio.input.transcription,
        TranscriptionSettings {
            model: "chirp_2".to_owned(),
            language: None,
        }
    );
    assert_eq!(created.session.audio.input.turn_detection, None);
}

#[test]
fn configure_command_asks_for_the_automatic_language() {
    let mut config = session_config(true);
    config.language = None;
    assert_eq!(
        config.configure_command().unwrap(),
        r#"{"kind":"configure","model":"chirp_2","language_codes":["auto"],"sample_rate_hertz":16000}"#
    );
}

#[test]
fn configure_command_carries_the_normalized_language() {
    assert_eq!(
        session_config(false).configure_command().unwrap(),
        r#"{"kind":"configure","model":"chirp_2","language_codes":["en-US"],"sample_rate_hertz":16000}"#
    );
}

#[test]
fn the_backend_commands_match_the_wire_format() {
    assert_eq!(FINISH_TURN_COMMAND, r#"{"kind":"finish_turn"}"#);
    assert_eq!(DISCARD_TURN_COMMAND, r#"{"kind":"discard_turn"}"#);
    assert_eq!(MAX_AUDIO_MESSAGE_BYTES, 25_000);
}

#[test]
fn configured_event_describes_the_session() {
    let mut transformer = chirp_transformer(true);
    let events = transformer
        .transform(ChirpBackendEvent::Configured)
        .unwrap();
    let documents = wire_events(&events);
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].event_type, "session.created");
    assert_eq!(documents[0].event_id, "event_0");
    let ChirpEvent::SessionCreated(created) = &events[0] else {
        panic!("a session.created event");
    };
    assert_eq!(created.session.id, "sess_1");
}

#[test]
fn backend_frames_decode_from_their_streaming_kinds() {
    let response: ChirpBackendEvent = serde_json::from_str(
        r#"{"kind":"response","speech_event":"begin","results":[{"transcript":"hello","is_final":false}],"billed_seconds":0.5}"#,
    )
    .unwrap();
    let mut transformer = chirp_transformer(true);
    let events = transformer.transform(response).unwrap();
    assert_eq!(event_types(&wire_events(&events)), [STARTED, DELTA]);
    let configured: ChirpBackendEvent = serde_json::from_str(r#"{"kind":"configured"}"#).unwrap();
    let created = transformer.transform(configured).unwrap();
    assert_eq!(event_types(&wire_events(&created)), ["session.created"]);
    let discarded: ChirpBackendEvent =
        serde_json::from_str(r#"{"kind":"turn_discarded","billed_seconds":1.5}"#).unwrap();
    assert!(transformer.transform(discarded).unwrap().is_empty());
    assert_eq!(
        transformer.take_unbilled_usage().unwrap(),
        Some(duration_usage(1.5))
    );
    let finished: ChirpBackendEvent = serde_json::from_str(r#"{"kind":"turn_finished"}"#).unwrap();
    assert!(transformer.transform(finished).unwrap().is_empty());
}

#[test]
fn an_unconfigured_transformer_rejects_backend_events() {
    let mut transformer =
        ChirpEventTransformer::new(FixedEventIds::default(), FixedItemIds::default());
    let error = transformer
        .transform(ChirpBackendEvent::Configured)
        .unwrap_err();
    assert!(matches!(error, ChirpProtocolError::Unconfigured));
    let frame_error = transformer
        .transform(frame(ChirpSpeechEvent::Begin, &[], "0"))
        .unwrap_err();
    assert!(matches!(frame_error, ChirpProtocolError::Unconfigured));
    assert_eq!(transformer.take_unbilled_usage().unwrap(), None);
}

#[test]
fn vad_interims_emit_begin_and_deltas_then_a_final_completes() {
    let mut transformer = chirp_transformer(true);
    let first_events = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", false)], "0.5"))
        .unwrap();
    let first = wire_events(&first_events);
    assert_eq!(event_types(&first), [STARTED, DELTA]);
    assert_eq!(first[0].item_id.as_deref(), Some("item_0"));
    assert_eq!(first[1].delta.as_deref(), Some("hello"));
    let second_events = transformer
        .transform(frame(
            ChirpSpeechEvent::Begin,
            &[("hello world", true)],
            "1.25",
        ))
        .unwrap();
    let second = wire_events(&second_events);
    assert_eq!(second.len(), 3);
    assert_eq!(event_types(&second), [DELTA, STOPPED, COMPLETED]);
    assert_eq!(second[0].delta.as_deref(), Some(" world"));
    assert_eq!(second[2].transcript.as_deref(), Some("hello world"));
    assert_eq!(second[2].reported_usage().usage_type, "duration");
    assert_eq!(
        second[2].reported_usage().seconds,
        Decimal::from_str("1.25").unwrap()
    );
}

#[test]
fn speech_started_is_emitted_once_per_turn() {
    let mut transformer = chirp_transformer(true);
    let first = transformer
        .transform(frame(ChirpSpeechEvent::Begin, &[("he", false)], "0.5"))
        .unwrap();
    assert_eq!(event_types(&wire_events(&first)), [STARTED, DELTA]);
    let second = transformer
        .transform(frame(ChirpSpeechEvent::Begin, &[("hello", false)], "0.5"))
        .unwrap();
    assert_eq!(event_types(&wire_events(&second)), [DELTA]);
}

#[test]
fn an_unchanged_interim_emits_nothing() {
    let mut transformer = chirp_transformer(true);
    let first = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", false)], "0.5"))
        .unwrap();
    assert_eq!(event_types(&wire_events(&first)), [STARTED, DELTA]);
    let second = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", false)], "0.5"))
        .unwrap();
    assert!(second.is_empty());
}

#[test]
fn a_final_matching_the_preview_emits_no_delta() {
    let mut transformer = chirp_transformer(true);
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", false)], "0.5"))
        .unwrap();
    let output = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", true)], "0.5"))
        .unwrap();
    let events = wire_events(&output);
    assert_eq!(event_types(&events), [STOPPED, COMPLETED]);
    assert_eq!(events[1].transcript.as_deref(), Some("hello"));
}

#[test]
fn a_final_before_any_interim_starts_a_new_turn() {
    let mut transformer = chirp_transformer(true);
    let first = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("one", true)], "1"))
        .unwrap();
    assert_eq!(
        event_types(&wire_events(&first)),
        [STARTED, DELTA, STOPPED, COMPLETED]
    );
    let second_events = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("two", false)], "1"))
        .unwrap();
    let second = wire_events(&second_events);
    assert_eq!(event_types(&second), [STARTED, DELTA]);
    assert_eq!(second[0].item_id.as_deref(), Some("item_1"));
    assert_eq!(second[1].delta.as_deref(), Some("two"));
}

#[test]
fn a_begin_frame_without_results_starts_the_turn() {
    let mut transformer = chirp_transformer(true);
    let output = transformer
        .transform(frame(ChirpSpeechEvent::Begin, &[], "0.5"))
        .unwrap();
    let events = wire_events(&output);
    assert_eq!(event_types(&events), [STARTED]);
    assert_eq!(events[0].item_id.as_deref(), Some("item_0"));
}

#[test]
fn one_frame_emits_begin_finals_interim_and_end_in_order() {
    let mut transformer = chirp_transformer(true);
    let output = transformer
        .transform(frame(
            ChirpSpeechEvent::End,
            &[("hello", true), ("next", false)],
            "0.5",
        ))
        .unwrap();
    let events = wire_events(&output);
    assert_eq!(
        event_types(&events),
        [STARTED, DELTA, STOPPED, COMPLETED, STARTED, DELTA, STOPPED]
    );
    assert_eq!(events[6].item_id.as_deref(), Some("item_1"));
}

#[test]
fn a_frame_with_an_interim_and_an_end_keeps_the_turn_open() {
    let mut transformer = chirp_transformer(true);
    let begun = transformer
        .transform(frame(ChirpSpeechEvent::Begin, &[("he", false)], "0.5"))
        .unwrap();
    let events = wire_events(&begun);
    assert_eq!(event_types(&events), [STARTED, DELTA]);
    let ended_events = transformer
        .transform(frame(ChirpSpeechEvent::End, &[("hello", false)], "0.5"))
        .unwrap();
    let ended = wire_events(&ended_events);
    assert_eq!(event_types(&ended), [DELTA, STOPPED]);
    let finished_events = transformer
        .transform(ChirpBackendEvent::TurnFinished)
        .unwrap();
    let finished = wire_events(&finished_events);
    assert_eq!(event_types(&finished), [COMPLETED]);
    assert_eq!(finished[0].transcript.as_deref(), Some("hello"));
}

#[test]
fn manual_mode_accumulates_finals_until_finish_turn() {
    let mut transformer = chirp_transformer(false);
    let first = transformer
        .transform(frame(ChirpSpeechEvent::Begin, &[("hello", true)], "0.5"))
        .unwrap();
    assert_eq!(event_types(&wire_events(&first)), [DELTA]);
    let second_events = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("world", true)], "1.5"))
        .unwrap();
    let second = wire_events(&second_events);
    assert_eq!(event_types(&second), [DELTA]);
    assert_eq!(second[0].delta.as_deref(), Some(" world"));
    let finished_events = transformer
        .transform(ChirpBackendEvent::TurnFinished)
        .unwrap();
    let finished = wire_events(&finished_events);
    assert_eq!(event_types(&finished), [COMPLETED]);
    assert_eq!(finished[0].transcript.as_deref(), Some("hello world"));
    assert_eq!(
        finished[0].reported_usage().seconds,
        Decimal::from_str("1.5").unwrap()
    );
}

#[test]
fn a_turn_finished_without_committed_text_completes_the_preview() {
    let mut transformer = chirp_transformer(false);
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("draft", false)], "0.5"))
        .unwrap();
    let finished_events = transformer
        .transform(ChirpBackendEvent::TurnFinished)
        .unwrap();
    let finished = wire_events(&finished_events);
    assert_eq!(finished[0].transcript.as_deref(), Some("draft"));
    assert_eq!(
        finished[0].reported_usage().seconds,
        Decimal::from_str("0.5").unwrap()
    );
}

#[test]
fn a_finished_turn_without_a_turn_emits_nothing() {
    let mut transformer = chirp_transformer(true);
    assert!(
        transformer
            .transform(ChirpBackendEvent::TurnFinished)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_end_frame_without_a_turn_emits_nothing() {
    let mut transformer = chirp_transformer(true);
    assert!(
        transformer
            .transform(frame(ChirpSpeechEvent::End, &[], "0.5"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_discarded_turn_keeps_the_largest_bill_and_drops_the_text() {
    let mut transformer = chirp_transformer(true);
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", false)], "2"))
        .unwrap();
    let discarded = ChirpTurnDiscarded {
        billed_seconds: Decimal::from_str("1.5").unwrap(),
    };
    assert!(
        transformer
            .transform(ChirpBackendEvent::TurnDiscarded(discarded))
            .unwrap()
            .is_empty()
    );
    assert!(
        transformer
            .transform(ChirpBackendEvent::TurnFinished)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        transformer.take_unbilled_usage().unwrap(),
        Some(duration_usage(2.0))
    );
}

#[test]
fn a_discarded_turn_can_raise_the_bill() {
    let mut transformer = chirp_transformer(true);
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", false)], "1"))
        .unwrap();
    let discarded = ChirpTurnDiscarded {
        billed_seconds: Decimal::from_str("3").unwrap(),
    };
    transformer
        .transform(ChirpBackendEvent::TurnDiscarded(discarded))
        .unwrap();
    assert_eq!(
        transformer.take_unbilled_usage().unwrap(),
        Some(duration_usage(3.0))
    );
}

#[test]
fn unbilled_seconds_are_reported_once_and_only_the_difference_afterwards() {
    let mut transformer = chirp_transformer(true);
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("he", false)], "0.5"))
        .unwrap();
    assert_eq!(
        transformer.take_unbilled_usage().unwrap(),
        Some(duration_usage(0.5))
    );
    assert_eq!(transformer.take_unbilled_usage().unwrap(), None);
    let finished_events = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", true)], "1.5"))
        .unwrap();
    let finished = wire_events(&finished_events);
    assert_eq!(
        finished[2].reported_usage().seconds,
        Decimal::from_str("1").unwrap()
    );
}

#[test]
fn usage_is_reported_once_per_completion() {
    let mut transformer = chirp_transformer(true);
    let output = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", true)], "1.25"))
        .unwrap();
    let events = wire_events(&output);
    assert_eq!(
        events[3].reported_usage().seconds,
        Decimal::from_str("1.25").unwrap()
    );
    assert_eq!(transformer.take_unbilled_usage().unwrap(), None);
}

#[test]
fn billing_reports_exact_decimal_seconds() {
    let mut transformer = chirp_transformer(true);
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", false)], "0.1"))
        .unwrap();
    assert_eq!(
        transformer.take_unbilled_usage().unwrap(),
        Some(duration_usage(0.1))
    );
    let output = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", true)], "0.3"))
        .unwrap();
    let events = wire_events(&output);
    assert_eq!(event_types(&events), [STOPPED, COMPLETED]);
    assert_eq!(
        events[1].reported_usage().seconds,
        Decimal::from_str("0.2").unwrap()
    );
}

#[test]
fn a_completion_without_billing_carries_no_usage() {
    let mut transformer = chirp_transformer(true);
    let output = transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", true)], "0"))
        .unwrap();
    let events = wire_events(&output);
    assert!(events[3].usage.is_none());
}

#[test]
fn billing_never_moves_backwards() {
    let mut transformer = chirp_transformer(false);
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("hello", true)], "2"))
        .unwrap();
    transformer
        .transform(frame(ChirpSpeechEvent::None, &[("there", true)], "1"))
        .unwrap();
    let finished_events = transformer
        .transform(ChirpBackendEvent::TurnFinished)
        .unwrap();
    let finished = wire_events(&finished_events);
    assert_eq!(finished[0].transcript.as_deref(), Some("hello there"));
    assert_eq!(
        finished[0].reported_usage().seconds,
        Decimal::from_str("2").unwrap()
    );
}

#[test]
fn every_final_completes_in_vad_mode() {
    let mut transformer = chirp_transformer(true);
    let output = transformer
        .transform(frame(
            ChirpSpeechEvent::None,
            &[("one", true), ("two", true)],
            "1",
        ))
        .unwrap();
    let events = wire_events(&output);
    assert_eq!(
        event_types(&events),
        [
            STARTED, DELTA, STOPPED, COMPLETED, STARTED, DELTA, STOPPED, COMPLETED
        ]
    );
    assert_eq!(events[3].transcript.as_deref(), Some("one"));
    assert_eq!(
        events[3].reported_usage().seconds,
        Decimal::from_str("1").unwrap()
    );
    assert_eq!(events[7].transcript.as_deref(), Some("two"));
    assert!(events[7].usage.is_none());
}

#[test]
fn blank_results_are_skipped_and_transcripts_are_trimmed() {
    let mut transformer = chirp_transformer(true);
    let output = transformer
        .transform(frame(
            ChirpSpeechEvent::None,
            &[("  hello  ", true), ("   ", false)],
            "1",
        ))
        .unwrap();
    let events = wire_events(&output);
    assert_eq!(event_types(&events), [STARTED, DELTA, STOPPED, COMPLETED]);
    assert_eq!(events[1].delta.as_deref(), Some("hello"));
    assert_eq!(events[3].transcript.as_deref(), Some("hello"));
    let interim_events = transformer
        .transform(frame(
            ChirpSpeechEvent::None,
            &[("hello", false), ("   ", false)],
            "1",
        ))
        .unwrap();
    let interim = wire_events(&interim_events);
    assert_eq!(event_types(&interim), [STARTED, DELTA]);
    assert_eq!(interim[1].delta.as_deref(), Some("hello"));
}

#[test]
fn blank_interims_emit_nothing() {
    let mut transformer = chirp_transformer(true);
    let results = [("  ", false), ("   ", false)];
    let events = transformer
        .transform(frame(ChirpSpeechEvent::None, &results, "0.5"))
        .unwrap();
    assert!(events.is_empty());
}

#[test]
fn uuid_item_ids_look_like_openai_item_ids() {
    let mut item_ids = UuidItemIds;
    let first = item_ids.next_item_id();
    let second = item_ids.next_item_id();
    assert_ne!(first, second);
    assert_item_id_is_uuid(&first);
    assert_item_id_is_uuid(&second);
}

fn assert_item_id_is_uuid(identifier: &str) {
    let suffix = identifier.strip_prefix("item_").expect("item prefix");
    assert_eq!(suffix.len(), 32);
    assert!(
        suffix
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
}
