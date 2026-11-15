#![cfg(test)]

use std::sync::Arc;
use std::sync::PoisonError;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;

use super::MetaRealtimeConfig;
use super::is_setup_message;
use crate::api::api_boundaries::ApiLimits;
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::realtime_contracts::events::AllDeltaType;
use crate::api::realtime_contracts::events::RealtimeResponseTransformInput;
use crate::api::realtime_contracts::events::RealtimeResponseTypedDict;
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::transcription_protocol::usage::DurationUsage;
use crate::api::transcription_protocol::usage::TranscriptionUsage;
use crate::meta_muse_realtime::END_STREAM;
use crate::meta_muse_realtime::MUSE_MODEL;
use crate::meta_muse_realtime::MuseProtocolError;
use crate::meta_muse_realtime::support::FixedIds;
use crate::meta_muse_realtime::support::capture;
use crate::meta_muse_realtime::support::captured_messages;
use crate::meta_muse_realtime::support::pacing_fakes;

const IDS: [&str; 8] = [
    "event_1", "event_2", "event_3", "event_4", "event_5", "event_6", "event_7", "event_8",
];

const SESSION: &str = r#"{"type":"session.update","session":{"type":"transcription","audio":{"input":{"format":{"type":"audio/pcm","rate":24000,"channels":1},"transcription":{"model":"muse-voice-transcribe-1.0","language":"es"}}}}}"#;
const HANDSHAKE: &str = r#"{"authorization":{"accessToken":"Bearer test-key"},"audioEncoding":"PCM_24KHZ","model":"muse-voice-transcribe-1.0","mode":"ENDPOINTING","partialMode":"CUMULATIVE","emitAudioProgress":true,"languageBias":["Spanish"]}"#;
const ENV_HANDSHAKE: &str = r#"{"authorization":{"accessToken":"Bearer env-key"},"audioEncoding":"PCM_24KHZ","model":"muse-voice-transcribe-1.0","mode":"ENDPOINTING","partialMode":"CUMULATIVE","emitAudioProgress":true,"languageBias":["Spanish"]}"#;
const PUSH_TO_TALK: &str = r#"{"type":"session.update","session":{"type":"realtime","audio":{"input":{"transcription":{}}},"turn_detection":null}}"#;
const PUSH_TO_TALK_HANDSHAKE: &str = r#"{"authorization":{"accessToken":"Bearer test-key"},"audioEncoding":"PCM_24KHZ","model":"muse-voice-transcribe-1.0","mode":"PUSH_TO_TALK","partialMode":"CUMULATIVE","emitAudioProgress":true}"#;
const CREATED: &str = r#"{"type":"session.created","event_id":"event_1","session":{"id":"session-42","object":"realtime.transcription_session","type":"transcription","audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"muse-voice-transcribe-1.0"},"turn_detection":{"type":"server_vad"}}}}}"#;
const PROVIDER_CREATED: &str = r#"{"type":"session.created","event_id":"event_1","session":{"id":"provider-7","object":"realtime.transcription_session","type":"transcription","audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"muse-voice-transcribe-1.0","language":"Spanish"},"turn_detection":{"type":"server_vad"}}}}}"#;
const PUSH_TO_TALK_CREATED: &str = r#"{"type":"session.created","event_id":"event_1","session":{"id":"provider-8","object":"realtime.transcription_session","type":"transcription","audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"muse-voice-transcribe-1.0"},"turn_detection":null}}}}"#;
const SPEECH_STARTED: &str =
    r#"[{"type":"input_audio_buffer.speech_started","event_id":"event_1","item_id":"turn-1"}]"#;
const BINARY_SPEECH_STARTED: &str =
    r#"[{"type":"input_audio_buffer.speech_started","event_id":"event_1","item_id":"turn-2"}]"#;
const PROVIDER_ERROR: &str = r#"[{"type":"error","error":{"type":"server_error","message":"Meta Muse realtime transcription failed"}}]"#;

fn base_config(ticks: &[f64]) -> MetaRealtimeConfig<FixedIds> {
    let fakes = pacing_fakes(ticks);
    MetaRealtimeConfig::new(
        fakes.clock,
        fakes.sleep,
        ApiLimits::default(),
        FixedIds::from(IDS.as_slice()),
    )
}

fn validate(
    config: &mut MetaRealtimeConfig<FixedIds>,
    api_key: Option<&str>,
    env_api_key: Option<&str>,
) -> Result<HeaderMap, MuseProtocolError> {
    let headers = HeaderMap::new();
    config.validate_environment(headers, MUSE_MODEL, api_key, env_api_key)
}

fn configure(config: &mut MetaRealtimeConfig<FixedIds>, payload: &str) -> Vec<RealtimeMessage> {
    config
        .transform_realtime_request(payload, MUSE_MODEL, None)
        .unwrap()
}

fn configured_config(ticks: &[f64]) -> MetaRealtimeConfig<FixedIds> {
    let mut config = base_config(ticks);
    let validated = validate(&mut config, Some("test-key"), None);
    assert!(validated.is_ok());
    let frames = configure(&mut config, SESSION);
    assert_eq!(frames, vec![RealtimeMessage::Text(HANDSHAKE.to_owned())]);
    config
}

fn transform_input() -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        session_configuration_request: Some(r#"{"type":"session.update"}"#.to_owned()),
        current_output_item_id: Some("item-1".to_owned()),
        current_response_id: Some("resp-1".to_owned()),
        current_delta_chunks: Some(vec![Json::Text("delta".to_owned())]),
        current_item_chunks: Some(vec![Json::Text("item".to_owned())]),
        current_conversation_id: Some("conv-1".to_owned()),
        current_delta_type: Some(AllDeltaType::Text),
    }
}

fn transform_response(
    config: &mut MetaRealtimeConfig<FixedIds>,
    message: &RealtimeMessage,
) -> Result<RealtimeResponseTypedDict, MuseProtocolError> {
    let input = transform_input();
    config.transform_realtime_response(message, MUSE_MODEL, &input)
}

fn rendered(typed: &RealtimeResponseTypedDict) -> String {
    serde_json::to_string(&typed.response).unwrap()
}

fn text_frame(payload: &str) -> RealtimeMessage {
    RealtimeMessage::Text(payload.to_owned())
}

fn append_message(audio: &[u8]) -> String {
    let encoded = STANDARD.encode(audio);
    format!(r#"{{"type":"input_audio_buffer.append","audio":"{encoded}"}}"#)
}

fn progress(config: &mut MetaRealtimeConfig<FixedIds>, milliseconds: i64) -> Json {
    let frame = format!(r#"{{"type":"audioProgress","audioProcessedMs":{milliseconds}}}"#);
    let typed = transform_response(config, &text_frame(&frame)).unwrap();
    typed.response
}

#[test]
fn validates_the_api_key_from_the_argument_or_the_environment() {
    let mut env_config = base_config(&[]);
    let headers = HeaderMap::from([("x-test".to_owned(), Json::Text("1".to_owned()))]);
    let returned = env_config
        .validate_environment(headers.clone(), MUSE_MODEL, None, Some("env-key"))
        .unwrap();
    assert_eq!(returned, headers);
    let frames = configure(&mut env_config, SESSION);
    assert_eq!(
        frames,
        vec![RealtimeMessage::Text(ENV_HANDSHAKE.to_owned())]
    );

    let mut missing = base_config(&[]);
    let rejected = validate(&mut missing, None, None).unwrap_err();
    assert!(matches!(rejected, MuseProtocolError::ApiKeyMissing));
    let unvalidated = missing
        .transform_realtime_request(SESSION, MUSE_MODEL, None)
        .unwrap_err();
    assert!(matches!(unvalidated, MuseProtocolError::TokenNotValidated));
    let blank = validate(&mut missing, Some("   "), Some("env-key")).unwrap_err();
    assert!(matches!(blank, MuseProtocolError::ApiKeyRequired));
}

#[test]
fn builds_the_default_created_event_with_fixed_ids() {
    let mut config = base_config(&[]);
    let event = config.transform_session_created_event("session-42");
    let text = serde_json::to_string(&event).unwrap();
    assert_eq!(text, CREATED);
}

#[test]
fn recognizes_setup_messages_by_authorization() {
    let setup = serde_json::from_str::<Json>(r#"{"authorization":{"accessToken":"x"}}"#).unwrap();
    assert!(is_setup_message(&setup));
    let plain = serde_json::from_str::<Json>(r#"{"type":"session.update"}"#).unwrap();
    assert!(!is_setup_message(&plain));
    assert!(!is_setup_message(&Json::Null));
    assert!(!is_setup_message(&Json::Text("authorization".to_owned())));
}

#[test]
fn configures_the_session_once_and_ignores_later_updates() {
    let (captured, _guard) = capture();
    let mut config = configured_config(&[]);
    let repeated = configure(&mut config, PUSH_TO_TALK);
    assert!(repeated.is_empty());
    assert_eq!(
        captured_messages(&captured),
        vec!["Meta realtime: ignoring session.update after the Muse handshake was sent".to_owned()]
    );
}

#[test]
fn accepts_transcription_session_updates() {
    let mut config = base_config(&[]);
    assert!(validate(&mut config, Some("test-key"), None).is_ok());
    let payload = r#"{"type":"transcription_session.update","session":{"type":"transcription","input_audio_format":"pcm16","input_audio_transcription":{"model":"muse-voice-transcribe-1.0","language":"es"}}}"#;
    let frames = configure(&mut config, payload);
    assert_eq!(frames, vec![RealtimeMessage::Text(HANDSHAKE.to_owned())]);
}

#[test]
fn rejects_a_session_update_that_changes_the_model() {
    let mut config = base_config(&[]);
    assert!(validate(&mut config, Some("test-key"), None).is_ok());
    let changed = r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"meta/other"}}}}}"#;
    let rejected = config
        .transform_realtime_request(changed, MUSE_MODEL, None)
        .unwrap_err();
    assert!(matches!(rejected, MuseProtocolError::ModelChanged));
}

#[test]
fn packetizes_appended_audio_into_80ms_frames() {
    let mut config = configured_config(&[]);
    let first: Vec<u8> = (0..3_840_u32).map(|index| index.to_le_bytes()[0]).collect();
    let tail: Vec<u8> = (0..100_u32).map(|index| index.to_le_bytes()[0]).collect();
    let full = configure(&mut config, &append_message(&first));
    assert_eq!(full, vec![RealtimeMessage::Binary(first)]);
    let buffered = configure(&mut config, &append_message(&tail));
    assert!(buffered.is_empty());
    let committed = configure(&mut config, r#"{"type":"input_audio_buffer.commit"}"#);
    assert_eq!(committed, vec![RealtimeMessage::Binary(tail)]);
    let ended = configure(&mut config, r#"{"type":"input_audio_buffer.end"}"#);
    assert_eq!(ended, vec![RealtimeMessage::Text(END_STREAM.to_owned())]);
    let repeated = configure(&mut config, r#"{"type":"input_audio_buffer.end"}"#);
    assert!(repeated.is_empty());
}

#[test]
fn flushes_push_to_talk_commits_with_the_end_stream() {
    let mut config = base_config(&[]);
    assert!(validate(&mut config, Some("test-key"), None).is_ok());
    let frames = configure(&mut config, PUSH_TO_TALK);
    assert_eq!(
        frames,
        vec![RealtimeMessage::Text(PUSH_TO_TALK_HANDSHAKE.to_owned())]
    );
    let audio = vec![7_u8; 100];
    let buffered = configure(&mut config, &append_message(&audio));
    assert!(buffered.is_empty());
    let committed = configure(&mut config, r#"{"type":"input_audio_buffer.commit"}"#);
    assert_eq!(
        committed,
        vec![
            RealtimeMessage::Binary(audio),
            RealtimeMessage::Text(END_STREAM.to_owned()),
        ]
    );
    let repeated = configure(&mut config, r#"{"type":"input_audio_buffer.commit"}"#);
    assert!(repeated.is_empty());
}

#[test]
fn drops_buffered_audio_on_clear() {
    let mut config = configured_config(&[]);
    let audio = vec![1_u8; 100];
    let buffered = configure(&mut config, &append_message(&audio));
    assert!(buffered.is_empty());
    let cleared = configure(&mut config, r#"{"type":"input_audio_buffer.clear"}"#);
    assert!(cleared.is_empty());
    let committed = configure(&mut config, r#"{"type":"input_audio_buffer.commit"}"#);
    assert!(committed.is_empty());
}

#[test]
fn rejects_audio_before_the_session_is_configured() {
    let mut config = base_config(&[]);
    let append = config
        .transform_realtime_request(&append_message(&[1, 2]), MUSE_MODEL, None)
        .unwrap_err();
    assert!(matches!(append, MuseProtocolError::SessionNotConfigured));
    let commit = config
        .transform_realtime_request(r#"{"type":"input_audio_buffer.commit"}"#, MUSE_MODEL, None)
        .unwrap_err();
    assert!(matches!(commit, MuseProtocolError::SessionNotConfigured));
}

#[test]
fn drops_unsupported_client_events_with_a_debug_line() {
    let (captured, _guard) = capture();
    let mut config = configured_config(&[]);
    let unknown = configure(&mut config, r#"{"type":"conversation.item.create"}"#);
    assert!(unknown.is_empty());
    let typeless = configure(&mut config, r#"{"type":null}"#);
    assert!(typeless.is_empty());
    assert_eq!(
        captured_messages(&captured),
        vec![
            "Meta realtime: dropping unsupported client event conversation.item.create".to_owned(),
            "Meta realtime: dropping unsupported client event".to_owned(),
        ]
    );
}

#[test]
fn rejects_malformed_client_messages() {
    let mut config = base_config(&[]);
    let text = config
        .transform_realtime_request("not json", MUSE_MODEL, None)
        .unwrap_err();
    assert!(matches!(text, MuseProtocolError::Protocol(_)));
    let array = config
        .transform_realtime_request("[]", MUSE_MODEL, None)
        .unwrap_err();
    assert!(matches!(array, MuseProtocolError::Protocol(_)));
    let mut configured = configured_config(&[]);
    let mute = configured
        .transform_realtime_request(r#"{"type":"input_audio_buffer.append"}"#, MUSE_MODEL, None)
        .unwrap_err();
    assert!(matches!(mute, MuseProtocolError::Protocol(_)));
}

#[tokio::test]
async fn paces_backend_sends_against_the_sent_duration() {
    let ticks = [1_000.0, 1_000.0, 1_000.25, 1_001.0, 1_001.0, 1_001.0];
    let fakes = pacing_fakes(&ticks);
    let delays = Arc::clone(&fakes.delays);
    let mut config = MetaRealtimeConfig::new(
        fakes.clock,
        fakes.sleep,
        ApiLimits::default(),
        FixedIds::from(IDS.as_slice()),
    );
    assert!(validate(&mut config, Some("test-key"), None).is_ok());
    let frames = configure(&mut config, SESSION);
    assert_eq!(frames, vec![RealtimeMessage::Text(HANDSHAKE.to_owned())]);
    let message = vec![0_u8; 12_000];
    config.pace_backend_send(&message).await.unwrap();
    config.pace_backend_send(&message).await.unwrap();
    config.pace_backend_send(&message).await.unwrap();
    config.pace_backend_send(&message).await.unwrap();
    config.pace_backend_send(&message).await.unwrap();
    config.pace_backend_send(&message).await.unwrap();
    let recorded = delays
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(recorded, vec![0.25_f64, 0.25, 0.25, 0.5]);
}

#[tokio::test]
async fn rejects_pacing_before_the_session_is_configured() {
    let fakes = pacing_fakes(&[5.0]);
    let mut config = MetaRealtimeConfig::new(
        fakes.clock,
        fakes.sleep,
        ApiLimits::default(),
        FixedIds::from(IDS.as_slice()),
    );
    let rejected = config.pace_backend_send(&[0_u8; 4_800]).await.unwrap_err();
    assert!(matches!(rejected, MuseProtocolError::SessionNotConfigured));
}

#[test]
fn reports_unbilled_usage_when_the_session_closes() {
    let mut config = configured_config(&[]);
    let invalid = transform_response(
        &mut config,
        &text_frame(r#"{"type":"audioProgress","audioProcessedMs":-2}"#),
    )
    .unwrap_err();
    assert!(matches!(invalid, MuseProtocolError::InvalidAudioProgress));
    assert_eq!(progress(&mut config, 1_250), Json::List(Vec::new()));
    assert_eq!(progress(&mut config, 1_250), Json::List(Vec::new()));
    assert_eq!(progress(&mut config, 2_000), Json::List(Vec::new()));
    let usage = config.unbilled_usage_on_session_close(MUSE_MODEL).unwrap();
    assert_eq!(
        usage,
        TranscriptionUsage::Duration(DurationUsage { seconds: 2.0 })
    );
    assert!(config.unbilled_usage_on_session_close(MUSE_MODEL).is_none());
}

#[test]
fn provider_frames_before_configuration_use_endpointing_mode() {
    let mut config = base_config(&[]);
    let orphan = transform_response(
        &mut config,
        &text_frame(r#"{"type":"transcript","transcript":"hi"}"#),
    )
    .unwrap_err();
    assert!(matches!(orphan, MuseProtocolError::MissingTurnId));
}

#[test]
fn passes_the_response_state_through_unchanged() {
    let mut config = configured_config(&[]);
    let input = transform_input();
    let typed = config
        .transform_realtime_response(&text_frame(r#"{"type":"unused"}"#), MUSE_MODEL, &input)
        .unwrap();
    assert_eq!(typed.response, Json::List(Vec::new()));
    assert_eq!(
        typed.session_configuration_request,
        input.session_configuration_request
    );
    assert_eq!(typed.current_output_item_id, input.current_output_item_id);
    assert_eq!(typed.current_response_id, input.current_response_id);
    assert_eq!(typed.current_delta_chunks, input.current_delta_chunks);
    assert_eq!(typed.current_conversation_id, input.current_conversation_id);
    assert_eq!(typed.current_item_chunks, input.current_item_chunks);
    assert_eq!(typed.current_delta_type, input.current_delta_type);
}

#[test]
fn transforms_provider_frames_into_event_lists() {
    let mut config = configured_config(&[]);
    let speech = transform_response(
        &mut config,
        &text_frame(r#"{"type":"speechStart","turnId":"turn-1"}"#),
    )
    .unwrap();
    assert_eq!(rendered(&speech), SPEECH_STARTED);
    let failure = transform_response(&mut config, &text_frame(r#"{"type":"error"}"#)).unwrap();
    assert_eq!(rendered(&failure), PROVIDER_ERROR);
    let ignored =
        transform_response(&mut config, &text_frame(r#"{"type":"sessionOpen"}"#)).unwrap();
    assert_eq!(rendered(&ignored), "[]");
    let null_session =
        transform_response(&mut config, &text_frame(r#"{"sessionId":null}"#)).unwrap();
    assert_eq!(rendered(&null_session), "[]");
}

#[test]
fn decodes_binary_provider_frames() {
    let mut config = configured_config(&[]);
    let message = RealtimeMessage::Binary(br#"{"type":"speechStart","turnId":"turn-2"}"#.to_vec());
    let speech = transform_response(&mut config, &message).unwrap();
    assert_eq!(rendered(&speech), BINARY_SPEECH_STARTED);
}

#[test]
fn builds_provider_created_events_from_the_configured_session() {
    let mut config = configured_config(&[]);
    let created =
        transform_response(&mut config, &text_frame(r#"{"sessionId":"provider-7"}"#)).unwrap();
    assert_eq!(rendered(&created), PROVIDER_CREATED);
    let mut push_to_talk = base_config(&[]);
    assert!(validate(&mut push_to_talk, Some("test-key"), None).is_ok());
    let frames = configure(&mut push_to_talk, PUSH_TO_TALK);
    assert_eq!(
        frames,
        vec![RealtimeMessage::Text(PUSH_TO_TALK_HANDSHAKE.to_owned())]
    );
    let created = transform_response(
        &mut push_to_talk,
        &text_frame(r#"{"sessionId":" provider-8 "}"#),
    )
    .unwrap();
    assert_eq!(rendered(&created), PUSH_TO_TALK_CREATED);
}

#[test]
fn rejects_invalid_handshake_responses() {
    let mut config = configured_config(&[]);
    let blank = transform_response(&mut config, &text_frame(r#"{"sessionId":"   "}"#)).unwrap_err();
    assert!(matches!(blank, MuseProtocolError::InvalidHandshake));
    let number = transform_response(&mut config, &text_frame(r#"{"sessionId":12}"#)).unwrap_err();
    assert!(matches!(number, MuseProtocolError::InvalidHandshake));
}

#[test]
fn rejects_non_utf8_and_malformed_provider_frames() {
    let mut config = configured_config(&[]);
    let binary =
        transform_response(&mut config, &RealtimeMessage::Binary(vec![0xff, 0xfe])).unwrap_err();
    assert!(matches!(binary, MuseProtocolError::FrameNotUtf8));
    let malformed = transform_response(&mut config, &text_frame("{")).unwrap_err();
    assert!(matches!(malformed, MuseProtocolError::Protocol(_)));
}
