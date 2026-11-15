#![cfg(test)]

use std::fs;
use std::path::Path;
use std::str::from_utf8;

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::api::api_boundaries::{ApiLimitError, ApiLimits};

use super::audio::decode_pcm16_append;
use super::events::{
    EventIdSource, SpeechEventType, UuidEventIds, completed_event, delta_event, error_event,
    speech_event, transcription_session, transcription_session_created_event,
};
use super::usage::{InputTokenDetails, TokenUsage, TranscriptionUsage, duration_usage};
use super::{
    RealtimeTranscriptionProtocolError, TranscriptionAudioFormat, TranscriptionLayout,
    TranscriptionSessionUpdate, json_object, parse_transcription_session_update,
};

fn parse(payload: &str) -> Result<TranscriptionSessionUpdate, RealtimeTranscriptionProtocolError> {
    parse_transcription_session_update::<RealtimeTranscriptionProtocolError>(
        payload.as_bytes(),
        ApiLimits::default(),
    )
}

fn rejection(payload: &str) -> String {
    parse(payload).unwrap_err().to_string()
}

fn audio_format(
    layout: TranscriptionLayout,
    encoding: Option<&str>,
    rate: Option<i64>,
    channels: Option<i64>,
) -> TranscriptionAudioFormat {
    TranscriptionAudioFormat {
        layout,
        encoding: encoding.map(str::to_owned),
        rate: rate.map(Number::from),
        channels: channels.map(Number::from),
    }
}

fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}

#[rstest]
#[case::beta_flat_pcm16(
    r#"{"type":"session.update","session":{"input_audio_format":"pcm16"}}"#,
    Some(audio_format(TranscriptionLayout::Beta, Some("pcm16"), None, None)),
    true
)]
#[case::beta_flat_custom(
    r#"{"type":"session.update","session":{"input_audio_format":"pcm24"}}"#,
    Some(audio_format(TranscriptionLayout::Beta, Some("pcm24"), None, None)),
    false
)]
#[case::ga_object_format(
    r#"{"type":"transcription_session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":24000,"channels":1}}}}}"#,
    Some(audio_format(TranscriptionLayout::Ga, Some("audio/pcm"), Some(24000), Some(1))),
    true
)]
#[case::ga_string_format(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":"custom"}}}}"#,
    Some(audio_format(TranscriptionLayout::Ga, Some("custom"), None, None)),
    false
)]
#[case::ga_object_without_encoding(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"rate":16000}}}}}"#,
    Some(audio_format(TranscriptionLayout::Ga, None, Some(16000), None)),
    false
)]
#[case::missing_format(
    r#"{"type":"session.update","session":{"type":"transcription"}}"#,
    None,
    false
)]
#[case::null_format(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":null}}}}"#,
    None,
    false
)]
fn transcription_session_layouts(
    #[case] payload: &str,
    #[case] expected: Option<TranscriptionAudioFormat>,
    #[case] pcm16: bool,
) {
    let update = parse(payload).unwrap();
    assert_eq!(update.audio_format, expected);
    assert_eq!(
        update.audio_format.is_some_and(|format| format.is_pcm16()),
        pcm16
    );
}

#[test]
fn transcription_session_transcription_fields() {
    let update = parse(
        r#"{"type":"session.update","session":{"input_audio_transcription":{"temperature":0.5,"prompt":"p","language":"en","model":"m"}}}"#,
    )
    .unwrap();
    assert_eq!(update.model.as_deref(), Some("m"));
    assert_eq!(update.language.as_deref(), Some("en"));
    assert_eq!(
        update.unsupported_transcription_keys,
        vec!["prompt".to_owned(), "temperature".to_owned()]
    );
    assert_eq!(update.session_type, None);
    let ga = parse(
        r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"m","language":"en","prompt":"p"}}}}}"#,
    )
    .unwrap();
    assert_eq!(ga.model.as_deref(), Some("m"));
    assert_eq!(ga.language.as_deref(), Some("en"));
    assert_eq!(ga.unsupported_transcription_keys, vec!["prompt".to_owned()]);
    let bare = parse(r#"{"type":"session.update","session":{"audio":{}}}"#).unwrap();
    assert_eq!(bare.model, None);
    assert_eq!(bare.language, None);
    assert!(bare.unsupported_transcription_keys.is_empty());
}

#[rstest]
#[case::absent(
    r#"{"type":"session.update","session":{"type":"transcription"}}"#,
    false,
    None
)]
#[case::explicit_null(
    r#"{"type":"session.update","session":{"turn_detection":null}}"#,
    true,
    None
)]
#[case::nested_null(
    r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":null}}}}"#,
    true,
    None
)]
#[case::server_vad(
    r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":{"type":"server_vad"}}}}}"#,
    false,
    Some("server_vad")
)]
#[case::flat_wins_over_nested(
    r#"{"type":"session.update","session":{"turn_detection":{"type":"server_vad"},"audio":{"input":{"turn_detection":null}}}}"#,
    false,
    Some("server_vad")
)]
#[case::flat_null_wins_over_nested(
    r#"{"type":"session.update","session":{"turn_detection":null,"audio":{"input":{"turn_detection":{"type":"server_vad"}}}}}"#,
    true,
    None
)]
#[case::nested_server_vad(
    r#"{"type":"session.update","session":{"audio":{"input":{"turn_detection":{"type":"server_vad"}}}}}"#,
    false,
    Some("server_vad")
)]
fn transcription_session_turn_detection(
    #[case] payload: &str,
    #[case] disabled: bool,
    #[case] detection_type: Option<&str>,
) {
    let update = parse(payload).unwrap();
    assert_eq!(update.turn_detection_disabled, disabled);
    assert_eq!(text(update.turn_detection_type()), detection_type);
}

#[test]
fn turn_detection_type_ignores_a_null_or_absent_type() {
    let null_type =
        parse(r#"{"type":"session.update","session":{"turn_detection":{"type":null}}}"#).unwrap();
    assert_eq!(null_type.turn_detection_type(), None);
    assert!(!null_type.turn_detection_disabled);
    let without_type =
        parse(r#"{"type":"session.update","session":{"turn_detection":{}}}"#).unwrap();
    assert_eq!(without_type.turn_detection_type(), None);
    assert!(!without_type.turn_detection_disabled);
}

#[test]
fn session_type_is_kept() {
    let update = parse(r#"{"type":"session.update","session":{"type":"transcription"}}"#).unwrap();
    assert_eq!(update.session_type.as_deref(), Some("transcription"));
}

#[test]
fn turn_detection_keeps_every_member() {
    let update = parse(
        r#"{"type":"session.update","session":{"turn_detection":{"type":"server_vad","threshold":0.5,"silence_duration_ms":500}}}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(update.turn_detection.as_ref().unwrap()).unwrap(),
        r#"{"type":"server_vad","threshold":0.5,"silence_duration_ms":500}"#
    );
}

#[rstest]
#[case::malformed("{", "invalid JSON object")]
#[case::empty("", "invalid JSON object")]
#[case::trailing("{} {}", "invalid JSON object")]
#[case::array("[]", "message must be a JSON object")]
#[case::scalar("7", "message must be a JSON object")]
#[case::other_type(r#"{"type":"other"}"#, "expected session.update")]
#[case::numeric_type(r#"{"type":7}"#, "expected session.update")]
#[case::list_type(r#"{"type":[]}"#, "expected session.update")]
#[case::object_type(r#"{"type":{}}"#, "expected session.update")]
#[case::missing_session(
    r#"{"type":"session.update"}"#,
    "session.update requires a session object"
)]
#[case::empty_session(
    r#"{"type":"session.update","session":{}}"#,
    "session.update requires a session object"
)]
#[case::null_session(
    r#"{"type":"session.update","session":null}"#,
    "session.update requires a session object"
)]
#[case::session_not_object(
    r#"{"type":"session.update","session":5}"#,
    "session must be an object"
)]
#[case::audio_not_object(
    r#"{"type":"session.update","session":{"audio":"x"}}"#,
    "session.audio must be an object"
)]
#[case::input_not_object(
    r#"{"type":"session.update","session":{"audio":{"input":true}}}"#,
    "session.audio.input must be an object"
)]
#[case::transcription_not_object(
    r#"{"type":"session.update","session":{"input_audio_transcription":"x"}}"#,
    "input audio transcription must be an object"
)]
#[case::turn_detection_not_object(
    r#"{"type":"session.update","session":{"turn_detection":[]}}"#,
    "turn_detection must be an object"
)]
#[case::format_not_object(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":7}}}}"#,
    "session.audio.input.format must be an object"
)]
#[case::session_type_not_string(
    r#"{"type":"session.update","session":{"type":5}}"#,
    "session.type must be a string"
)]
#[case::model_not_string(
    r#"{"type":"session.update","session":{"input_audio_transcription":{"model":1}}}"#,
    "transcription model must be a string"
)]
#[case::language_not_string(
    r#"{"type":"session.update","session":{"input_audio_transcription":{"language":true}}}"#,
    "language must be a string"
)]
#[case::beta_format_not_string(
    r#"{"type":"session.update","session":{"input_audio_format":5}}"#,
    "session.input_audio_format must be a string"
)]
#[case::format_type_not_string(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":5}}}}}"#,
    "session.audio.input.format.type must be a string"
)]
#[case::rate_boolean(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"rate":true}}}}}"#,
    "session.audio.input.format.rate must be an integer"
)]
#[case::rate_float(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"rate":1.5}}}}}"#,
    "session.audio.input.format.rate must be an integer"
)]
#[case::rate_string(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"rate":"1"}}}}}"#,
    "session.audio.input.format.rate must be an integer"
)]
#[case::channels_boolean(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"channels":false}}}}}"#,
    "session.audio.input.format.channels must be an integer"
)]
#[case::channels_float(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"channels":2.0}}}}}"#,
    "session.audio.input.format.channels must be an integer"
)]
#[case::channels_string(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"channels":"2"}}}}}"#,
    "session.audio.input.format.channels must be an integer"
)]
#[case::both_transcription_layouts(
    r#"{"type":"session.update","session":{"input_audio_transcription":{},"audio":{"input":{"transcription":{}}}}}"#,
    "input transcription must use either beta or GA layout"
)]
#[case::both_audio_format_layouts(
    r#"{"type":"session.update","session":{"input_audio_format":"pcm16","audio":{"input":{"format":"audio/pcm"}}}}"#,
    "input audio format must use either beta or GA layout"
)]
#[case::missing_type(
    r#"{"session":{"input_audio_format":"pcm16"}}"#,
    "expected session.update"
)]
#[case::rate_exponent(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"rate":1e5}}}}}"#,
    "session.audio.input.format.rate must be an integer"
)]
#[case::rate_exponent_upper(
    r#"{"type":"session.update","session":{"audio":{"input":{"format":{"rate":1E5}}}}}"#,
    "session.audio.input.format.rate must be an integer"
)]
fn transcription_protocol_errors(#[case] payload: &str, #[case] message: &str) {
    assert_eq!(rejection(payload), message);
}

#[test]
fn null_optional_values_pass() {
    let update = parse(
        r#"{"type":"session.update","session":{"type":null,"input_audio_transcription":{"model":null,"language":null},"input_audio_format":null,"turn_detection":null,"audio":{"input":{"format":null,"transcription":null}}}}"#,
    )
    .unwrap();
    assert_eq!(update.session_type, None);
    assert_eq!(update.model, None);
    assert_eq!(update.language, None);
    assert_eq!(update.audio_format, None);
    assert_eq!(update.turn_detection, None);
    assert!(update.turn_detection_disabled);
}

#[test]
fn integer_rates_pass_at_zero_and_below() {
    let update = parse(
        r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":0,"channels":-1}}}}}"#,
    )
    .unwrap();
    let format = update.audio_format.unwrap();
    assert_eq!(format.rate.and_then(|number| number.as_i64()), Some(0));
    assert_eq!(format.channels.and_then(|number| number.as_i64()), Some(-1));
}

#[test]
fn integer_rates_keep_values_past_i64() {
    let update = parse(
        r#"{"type":"session.update","session":{"audio":{"input":{"format":{"type":"audio/pcm","rate":1180591620717411303424}}}}}"#,
    )
    .unwrap();
    let rate = update.audio_format.unwrap().rate.unwrap();
    assert_eq!(rate.to_string(), "1180591620717411303424");
}

#[derive(Debug, PartialEq, Eq)]
enum CallerError {
    Protocol(RealtimeTranscriptionProtocolError),
}

impl From<RealtimeTranscriptionProtocolError> for CallerError {
    fn from(error: RealtimeTranscriptionProtocolError) -> Self {
        Self::Protocol(error)
    }
}

#[test]
fn caller_supplied_error_type_keeps_the_message() {
    let error =
        parse_transcription_session_update::<CallerError>(b"[]", ApiLimits::default()).unwrap_err();
    let CallerError::Protocol(inner) = error;
    assert_eq!(inner, RealtimeTranscriptionProtocolError::MessageNotObject);
    assert_eq!(inner.to_string(), "message must be a JSON object");
}

#[rstest]
#[case::padded_pair(r#""AAA=""#, None, Ok(vec![0, 0]))]
#[case::padded_single(r#""AA==""#, None, Err("PCM16 audio must contain complete samples"))]
#[case::full_quad(r#""AAAA""#, None, Err("PCM16 audio must contain complete samples"))]
#[case::two_full_quads(r#""AAAAAAAA""#, None, Ok(vec![0, 0, 0, 0, 0, 0]))]
#[case::empty(r#""""#, None, Ok(vec![]))]
#[case::trailing_bits(r#""AAB=""#, None, Ok(vec![0, 0]))]
#[case::null("null", None, Err("Audio must be a base64 string"))]
#[case::number("5", None, Err("Audio must be a base64 string"))]
#[case::boolean("true", None, Err("Audio must be a base64 string"))]
#[case::object("{}", None, Err("Audio must be a base64 string"))]
#[case::list("[]", None, Err("Audio must be a base64 string"))]
#[case::whitespace(r#""A A=""#, None, Err("Audio must be valid base64"))]
#[case::bad_alphabet(r#""ab-_""#, None, Err("Audio must be valid base64"))]
#[case::missing_padding(r#""AAA""#, None, Err("Audio must be valid base64"))]
#[case::excess_padding(r#""AA===""#, None, Err("Audio must be valid base64"))]
#[case::non_ascii("\"\u{e9}AA=\"", None, Err("Audio must be valid base64"))]
#[case::backlog(
    r#""AAAAAAAA""#,
    Some(4),
    Err("Audio append exceeds the four-second backlog limit")
)]
#[case::backlog_before_decoding(
    r#""A A A A A""#,
    Some(4),
    Err("Audio append exceeds the four-second backlog limit")
)]
#[case::limit_accepts_padded_pair(r#""AAA=""#, Some(4), Ok(vec![0, 0]))]
#[case::limit_accepts_quad(r#""AAAAAAAA""#, Some(8), Ok(vec![0, 0, 0, 0, 0, 0]))]
#[case::limit_counts_characters(
    "\"\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\"",
    Some(8),
    Err("Audio must be valid base64")
)]
fn pcm16_append_validation(
    #[case] value: &str,
    #[case] max_encoded_bytes: Option<usize>,
    #[case] expected: Result<Vec<u8>, &'static str>,
) {
    let audio: Json = serde_json::from_str(value).unwrap();
    let decoded =
        decode_pcm16_append::<RealtimeTranscriptionProtocolError>(Some(&audio), max_encoded_bytes);
    match expected {
        Ok(bytes) => assert_eq!(decoded.unwrap(), bytes),
        Err(message) => assert_eq!(decoded.unwrap_err().to_string(), message),
    }
}

const EVENT_IDS: [&str; 8] = [
    "event_00000000000000000000000000000001",
    "event_00000000000000000000000000000002",
    "event_00000000000000000000000000000003",
    "event_00000000000000000000000000000004",
    "event_00000000000000000000000000000005",
    "event_00000000000000000000000000000006",
    "event_00000000000000000000000000000007",
    "event_00000000000000000000000000000008",
];

struct FixedIds<'ids> {
    ids: &'ids [&'ids str],
    next: usize,
}

impl EventIdSource for FixedIds<'_> {
    fn next_event_id(&mut self) -> String {
        let id = self.ids.get(self.next).copied().unwrap_or_default();
        self.next = self.next.saturating_add(1);
        id.to_owned()
    }
}

const SESSION_JSON: &str = r#"{"id":"s","object":"realtime.transcription_session","type":"transcription","audio":{"input":{"format":{"type":"audio/pcm","rate":16000},"transcription":{"model":"m","language":"en"},"turn_detection":{"type":"server_vad"}}}}"#;
const BARE_SESSION_JSON: &str = r#"{"id":"s","object":"realtime.transcription_session","type":"transcription","audio":{"input":{"format":{"type":"audio/pcm","rate":16000},"transcription":{"model":"m"},"turn_detection":null}}}"#;
const STARTED_JSON: &str = r#"{"type":"input_audio_buffer.speech_started","event_id":"event_00000000000000000000000000000002","item_id":"i"}"#;
const STOPPED_JSON: &str = r#"{"type":"input_audio_buffer.speech_stopped","event_id":"event_00000000000000000000000000000003","item_id":"i"}"#;
const DELTA_JSON: &str = r#"{"type":"conversation.item.input_audio_transcription.delta","event_id":"event_00000000000000000000000000000004","item_id":"i","content_index":0,"delta":"he"}"#;
const SILENT_JSON: &str = r#"{"type":"conversation.item.input_audio_transcription.completed","event_id":"event_00000000000000000000000000000005","item_id":"i","content_index":0,"transcript":"hello"}"#;
const TIMED_JSON: &str = r#"{"type":"conversation.item.input_audio_transcription.completed","event_id":"event_00000000000000000000000000000006","item_id":"i","content_index":0,"transcript":"hello","usage":{"type":"duration","seconds":1.25}}"#;
const BILLED_JSON: &str = r#"{"type":"conversation.item.input_audio_transcription.completed","event_id":"event_00000000000000000000000000000007","item_id":"i","content_index":0,"transcript":"hello","usage":{"type":"tokens","input_tokens":3,"output_tokens":4,"total_tokens":7,"input_token_details":{"text_tokens":2,"audio_tokens":1}}}"#;
const ERROR_JSON: &str = r#"{"type":"error","error":{"type":"server_error","message":"denied"}}"#;

#[test]
fn transcription_event_shapes() {
    let mut ids = FixedIds {
        ids: &EVENT_IDS,
        next: 0,
    };
    let session = transcription_session("s", "m", 16000, Some("en"), true);
    assert_eq!(serde_json::to_string(&session).unwrap(), SESSION_JSON);
    let created = transcription_session_created_event(ids.next_event_id(), &session);
    let created_json = serde_json::to_value(&created).unwrap();
    assert_eq!(created_json["type"], "session.created");
    assert_eq!(created_json["event_id"], EVENT_IDS[0]);
    assert_eq!(
        created_json["session"],
        serde_json::to_value(serde_json::from_str::<Json>(SESSION_JSON).unwrap()).unwrap()
    );
    let bare = transcription_session("s", "m", 16000, None, false);
    assert_eq!(serde_json::to_string(&bare).unwrap(), BARE_SESSION_JSON);
}

#[test]
fn transcription_speech_and_delta_events() {
    let started = speech_event(EVENT_IDS[1].to_owned(), SpeechEventType::Started, "i");
    assert_eq!(serde_json::to_string(&started).unwrap(), STARTED_JSON);
    let stopped = speech_event(EVENT_IDS[2].to_owned(), SpeechEventType::Stopped, "i");
    assert_eq!(serde_json::to_string(&stopped).unwrap(), STOPPED_JSON);
    let delta = delta_event(EVENT_IDS[3].to_owned(), "i", "he");
    assert_eq!(serde_json::to_string(&delta).unwrap(), DELTA_JSON);
}

#[test]
fn transcription_completed_events_keep_their_usage() {
    let silent = completed_event(EVENT_IDS[4].to_owned(), "i", "hello", None);
    assert_eq!(serde_json::to_string(&silent).unwrap(), SILENT_JSON);
    let timed = completed_event(
        EVENT_IDS[5].to_owned(),
        "i",
        "hello",
        Some(duration_usage(1.25)),
    );
    assert_eq!(serde_json::to_string(&timed).unwrap(), TIMED_JSON);
    let tokens = TranscriptionUsage::Tokens(Box::new(TokenUsage {
        input_tokens: 3,
        output_tokens: 4,
        total_tokens: 7,
        input_token_details: InputTokenDetails {
            text_tokens: 2,
            audio_tokens: 1,
        },
    }));
    let billed = completed_event(EVENT_IDS[6].to_owned(), "i", "hello", Some(tokens));
    assert_eq!(serde_json::to_string(&billed).unwrap(), BILLED_JSON);
}

#[test]
fn transcription_error_event_shape() {
    assert_eq!(
        serde_json::to_string(&error_event("denied")).unwrap(),
        ERROR_JSON
    );
}

#[test]
fn event_ids_are_prefixed_lowercase_hex() {
    let mut ids = UuidEventIds;
    let identifier = ids.next_event_id();
    let hex = identifier.strip_prefix("event_").unwrap();
    assert_eq!(hex.len(), 32);
    assert!(
        hex.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn json_object_requires_an_object() {
    assert_eq!(
        json_object::<RealtimeTranscriptionProtocolError>(b"[]")
            .unwrap_err()
            .to_string(),
        "message must be a JSON object"
    );
    assert_eq!(
        json_object::<RealtimeTranscriptionProtocolError>(b"{")
            .unwrap_err()
            .to_string(),
        "invalid JSON object"
    );
}

#[test]
fn oversized_payloads_are_refused_before_parsing() {
    let limits = ApiLimits {
        max_json_bytes: 1024,
        max_event_bytes: 8,
        max_items: 4,
    };
    let error = parse_transcription_session_update::<RealtimeTranscriptionProtocolError>(
        b"{\"type\":x",
        limits,
    )
    .unwrap_err();
    assert_eq!(
        error,
        RealtimeTranscriptionProtocolError::Limit(ApiLimitError::EventTooLarge {
            observed: 9,
            limit: 8
        })
    );
}

#[rstest]
#[case::nan("NaN")]
#[case::infinity("Infinity")]
#[case::negative_infinity("-Infinity")]
fn non_finite_literals_are_invalid_json(#[case] literal: &str) {
    let payload = format!(
        r#"{{"type":"session.update","session":{{"input_audio_transcription":{{"temperature":{literal}}}}}}}"#
    );
    assert_eq!(rejection(&payload), "invalid JSON object");
}

#[rstest]
#[case::shallow(100, true)]
#[case::deep(200, false)]
fn nesting_depth_is_capped(#[case] depth: usize, #[case] accepted: bool) {
    let payload = format!(
        r#"{{"type":"session.update","session":{{"extra":{}{}}}}}"#,
        "[".repeat(depth),
        "]".repeat(depth)
    );
    let parsed = parse(&payload);
    assert_eq!(parsed.is_ok(), accepted);
    if !accepted {
        assert_eq!(parsed.unwrap_err().to_string(), "invalid JSON object");
    }
}

fn replay(bytes: &[u8]) {
    let Some((&selector, rest)) = bytes.split_first() else {
        return;
    };
    match selector {
        0 => drop(parse_transcription_session_update::<
            RealtimeTranscriptionProtocolError,
        >(rest, ApiLimits::default())),
        1 => {
            let Ok(text) = from_utf8(rest) else {
                return;
            };
            let audio = Json::Text(text.to_owned());
            drop(decode_pcm16_append::<RealtimeTranscriptionProtocolError>(
                Some(&audio),
                None,
            ));
        }
        _ => {}
    }
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/transcription_protocol");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| replay(&bytes))
        .count();
    assert!(replayed >= 8);
}
