#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use rstest::rstest;
use uuid::Uuid;

use super::MuseEvent;
use super::MuseEventTransformer;
use crate::api::transcription_protocol::JsonObject;
use crate::api::transcription_protocol::events::CompletedEvent;
use crate::api::transcription_protocol::events::SpeechEventType;
use crate::api::transcription_protocol::events::UuidEventIds;
use crate::api::transcription_protocol::json_object;
use crate::api::transcription_protocol::usage::DurationUsage;
use crate::api::transcription_protocol::usage::TranscriptionUsage;
use crate::meta_muse_realtime::MuseMode;
use crate::meta_muse_realtime::MuseProtocolError;
use crate::meta_muse_realtime::TURN_LIMIT;
use crate::meta_muse_realtime::support::CountingIds;

struct Harness {
    transformer: MuseEventTransformer,
    ids: CountingIds,
    mode: MuseMode,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            transformer: MuseEventTransformer::default(),
            ids: CountingIds::default(),
            mode: MuseMode::Endpointing,
        }
    }
}

impl Harness {
    fn transform(&mut self, payload: &str) -> Vec<MuseEvent> {
        let events = self.try_transform(payload);
        events.unwrap()
    }

    fn try_transform(&mut self, payload: &str) -> Result<Vec<MuseEvent>, MuseProtocolError> {
        let frame = parse_frame(payload);
        self.transformer.transform(&frame, self.mode, &mut self.ids)
    }

    fn rendered(&mut self, payload: &str) -> Vec<String> {
        let events = self.transform(payload);
        events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect()
    }
}

fn parse_frame(payload: &str) -> JsonObject {
    let parsed = serde_json::from_str::<Json>(payload).unwrap();
    let Json::Object(fields) = parsed else {
        panic!("the test frame is a JSON object");
    };
    fields
}

fn kinds(events: &[MuseEvent]) -> Vec<&'static str> {
    events.iter().map(kind_of).collect()
}

fn kind_of(event: &MuseEvent) -> &'static str {
    match event {
        MuseEvent::Created(_) => "created",
        MuseEvent::Error(_) => "error",
        MuseEvent::Speech(speech) => match speech.kind {
            SpeechEventType::Started => "started",
            SpeechEventType::Stopped => "stopped",
        },
        MuseEvent::Delta(_) => "delta",
        MuseEvent::Completed(_) => "completed",
    }
}

fn turn_item(event: &MuseEvent) -> &str {
    let item_id = match event {
        MuseEvent::Speech(speech) => &speech.item_id,
        MuseEvent::Delta(delta) => &delta.item_id,
        MuseEvent::Completed(completed) => &completed.item_id,
        MuseEvent::Created(_) | MuseEvent::Error(_) => panic!("a turn event"),
    };
    item_id.as_str()
}

fn delta_text(event: &MuseEvent) -> &str {
    let MuseEvent::Delta(delta) = event else {
        panic!("a delta event");
    };
    delta.delta.as_str()
}

fn completed_of(event: &MuseEvent) -> &CompletedEvent {
    let MuseEvent::Completed(completed) = event else {
        panic!("a completed event");
    };
    completed
}

fn started_kind(event: &MuseEvent) -> SpeechEventType {
    let MuseEvent::Speech(speech) = event else {
        panic!("a speech event");
    };
    speech.kind
}

#[test]
fn emits_speech_started_before_the_first_delta() {
    let mut harness = Harness::default();
    let started = harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#);
    assert_eq!(kinds(&started), vec!["started"]);
    assert_eq!(started_kind(&started[0]), SpeechEventType::Started);
    assert_eq!(turn_item(&started[0]), "t1");
    let partial = harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"hell"}"#);
    assert_eq!(kinds(&partial), vec!["delta"]);
    assert_eq!(turn_item(&partial[0]), "t1");
    assert_eq!(delta_text(&partial[0]), "hell");
}

#[test]
fn repeated_partials_emit_only_the_new_suffix() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#));
    let first = harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"hell"}"#);
    assert_eq!(delta_text(&first[0]), "hell");
    let repeated = harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"hell"}"#);
    assert!(repeated.is_empty());
    let extended = harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"hello"}"#);
    assert_eq!(delta_text(&extended[0]), "o");
    let shortened = harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"hell"}"#);
    assert!(shortened.is_empty());
    let recovered =
        harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"hello world"}"#);
    assert_eq!(delta_text(&recovered[0]), " world");
}

#[test]
fn final_transcripts_stop_the_turn_and_complete_it() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#));
    drop(harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"hello"}"#));
    let finished = harness.transform(
        r#"{"type":"transcript","turnId":"t1","transcript":"hello world","final":true}"#,
    );
    assert_eq!(kinds(&finished), vec!["stopped", "completed"]);
    assert_eq!(turn_item(&finished[0]), "t1");
    let completed = completed_of(&finished[1]);
    assert_eq!(completed.transcript, "hello world");
    assert_eq!(completed.content_index, 0);
    assert!(completed.usage.is_none());
}

#[test]
fn speech_complete_emits_started_stopped_and_completed() {
    let mut harness = Harness::default();
    let events =
        harness.transform(r#"{"type":"speechComplete","turnId":"t1","transcript":"done"}"#);
    assert_eq!(kinds(&events), vec!["started", "stopped", "completed"]);
    assert_eq!(completed_of(&events[2]).transcript, "done");
}

#[test]
fn repeated_end_events_do_not_repeat_turn_events() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"speechComplete","turnId":"t1","transcript":"done"}"#));
    let again = harness.transform(r#"{"type":"speechComplete","turnId":"t1","transcript":"done"}"#);
    assert!(again.is_empty());
    let ended = harness.transform(r#"{"type":"speechEnd","turnId":"t1"}"#);
    assert!(ended.is_empty());
    let restarted = harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#);
    assert!(restarted.is_empty());
}

#[test]
fn speech_end_stops_a_turn_without_completing_it() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#));
    let ended = harness.transform(r#"{"type":"speechEnd","turnId":"t1"}"#);
    assert_eq!(kinds(&ended), vec!["stopped"]);
    assert_eq!(turn_item(&ended[0]), "t1");
    let repeated = harness.transform(r#"{"type":"speechEnd","turnId":"t1"}"#);
    assert!(repeated.is_empty());
    assert!(harness.transformer.take_unbilled_usage().is_none());
}

#[test]
fn completed_turns_do_not_restart() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"speechComplete","turnId":"t1","transcript":"done"}"#));
    let restarted = harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#);
    assert!(restarted.is_empty());
    let orphan = harness
        .try_transform(r#"{"type":"transcript","transcript":"next"}"#)
        .unwrap_err();
    assert!(matches!(orphan, MuseProtocolError::MissingTurnId));
}

#[test]
fn separate_turns_keep_their_own_state() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#));
    drop(harness.transform(r#"{"type":"speechStart","turnId":"t2"}"#));
    let first = harness.transform(r#"{"type":"transcript","turnId":"t1","transcript":"one"}"#);
    assert_eq!(kinds(&first), vec!["delta"]);
    assert_eq!(turn_item(&first[0]), "t1");
    let second = harness.transform(r#"{"type":"transcript","turnId":"t2","transcript":"two"}"#);
    assert_eq!(kinds(&second), vec!["delta"]);
    assert_eq!(turn_item(&second[0]), "t2");
}

#[test]
fn transcripts_without_a_turn_id_use_the_active_turn() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#));
    let partial = harness.transform(r#"{"type":"transcript","transcript":"hi"}"#);
    assert_eq!(kinds(&partial), vec!["delta"]);
    assert_eq!(turn_item(&partial[0]), "t1");
    let finished =
        harness.transform(r#"{"type":"transcript","transcript":"hi there","final":true}"#);
    assert_eq!(kinds(&finished), vec!["stopped", "completed"]);
    assert_eq!(completed_of(&finished[1]).transcript, "hi there");
    let orphan = harness
        .try_transform(r#"{"type":"transcript","transcript":"next"}"#)
        .unwrap_err();
    assert!(matches!(orphan, MuseProtocolError::MissingTurnId));
}

#[test]
fn empty_transcripts_without_a_turn_or_active_turn_are_ignored() {
    let mut harness = Harness::default();
    let empty = harness.transform(r#"{"type":"transcript","transcript":""}"#);
    assert!(empty.is_empty());
    let explicit = harness.transform(r#"{"type":"transcript","turnId":"t2","transcript":""}"#);
    assert_eq!(kinds(&explicit), vec!["started"]);
    assert_eq!(turn_item(&explicit[0]), "t2");
}

#[test]
fn push_to_talk_transcripts_create_random_item_ids() {
    let mut harness = Harness {
        transformer: MuseEventTransformer::default(),
        ids: CountingIds::default(),
        mode: MuseMode::PushToTalk,
    };
    let events = harness.transform(r#"{"type":"transcript","transcript":"hi"}"#);
    assert_eq!(kinds(&events), vec!["started", "delta"]);
    let item_id = turn_item(&events[0]);
    let identifier = item_id.strip_prefix("item_").unwrap();
    assert_eq!(identifier.len(), 32);
    let parsed = Uuid::parse_str(identifier).unwrap();
    assert_eq!(parsed.get_version_num(), 4);
    assert_eq!(turn_item(&events[1]), item_id);
    drop(harness.transform(r#"{"type":"transcript","transcript":"hi there","final":true}"#));
    let next = harness.transform(r#"{"type":"transcript","transcript":"again"}"#);
    assert_eq!(kinds(&next), vec!["started", "delta"]);
    assert_ne!(turn_item(&next[0]), item_id);
}

#[test]
fn turns_beyond_the_limit_drop_the_oldest() {
    let mut harness = Harness::default();
    let starts: Vec<String> = (0..TURN_LIMIT)
        .map(|index| format!(r#"{{"type":"speechStart","turnId":"t{index}"}}"#))
        .collect();
    starts
        .iter()
        .map(|payload| harness.transform(payload))
        .for_each(drop);
    let recent = harness.transform(r#"{"type":"transcript","turnId":"t127","transcript":"y"}"#);
    assert_eq!(kinds(&recent), vec!["delta"]);
    let oldest = harness.transform(r#"{"type":"transcript","turnId":"t0","transcript":"x"}"#);
    assert_eq!(kinds(&oldest), vec!["delta"]);
    let newest = harness.transform(r#"{"type":"speechStart","turnId":"t128"}"#);
    assert_eq!(kinds(&newest), vec!["started"]);
    let evicted = harness.transform(r#"{"type":"transcript","turnId":"t0","transcript":"x"}"#);
    assert_eq!(kinds(&evicted), vec!["started", "delta"]);
    assert_eq!(turn_item(&evicted[0]), "t0");
    assert_eq!(delta_text(&evicted[1]), "x");
}

#[test]
fn audio_progress_accumulates_seconds_and_resets() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"audioProgress","audioProcessedMs":500}"#));
    assert_eq!(
        harness.transformer.take_unbilled_usage(),
        Some(TranscriptionUsage::Duration(DurationUsage { seconds: 0.5 }))
    );
    drop(harness.transform(r#"{"type":"audioProgress","audioProcessedMs":500}"#));
    drop(harness.transform(r#"{"type":"audioProgress","audioProcessedMs":499}"#));
    drop(harness.transform(r#"{"type":"audioProgress","audioProcessedMs":1500}"#));
    assert_eq!(
        harness.transformer.take_unbilled_usage(),
        Some(TranscriptionUsage::Duration(DurationUsage { seconds: 1.0 }))
    );
    drop(harness.transform(r#"{"type":"audioProgress","audioProcessedMs":1750}"#));
    assert_eq!(
        harness.transformer.take_unbilled_usage(),
        Some(TranscriptionUsage::Duration(DurationUsage {
            seconds: 0.25
        }))
    );
    assert!(harness.transformer.take_unbilled_usage().is_none());
}

#[test]
fn invalid_audio_progress_is_an_error() {
    let mut harness = Harness::default();
    let invalid = [
        r#"{"type":"audioProgress"}"#,
        r#"{"type":"audioProgress","audioProcessedMs":null}"#,
        r#"{"type":"audioProgress","audioProcessedMs":true}"#,
        r#"{"type":"audioProgress","audioProcessedMs":"1000"}"#,
        r#"{"type":"audioProgress","audioProcessedMs":-1}"#,
    ];
    let errors: Vec<MuseProtocolError> = invalid
        .iter()
        .map(|payload| harness.try_transform(payload).unwrap_err())
        .collect();
    assert!(
        errors
            .iter()
            .all(|error| matches!(error, MuseProtocolError::InvalidAudioProgress))
    );
}

#[test]
fn unknown_frames_and_provider_errors() {
    let mut harness = Harness::default();
    assert!(harness.transform(r#"{"type":"sessionOpen"}"#).is_empty());
    assert!(harness.transform("{}").is_empty());
    let errors = harness.rendered(r#"{"type":"error","message":"provider secret"}"#);
    assert_eq!(
        errors,
        vec![
            r#"{"type":"error","error":{"type":"server_error","message":"Meta Muse realtime transcription failed"}}"#
                .to_owned()
        ]
    );
}

#[test]
fn integer_turn_ids_render_the_way_python_prints_them() {
    let mut harness = Harness::default();
    let negative_zero = harness.transform(r#"{"type":"speechStart","turnId":-0}"#);
    assert_eq!(turn_item(&negative_zero[0]), "0");
    let large =
        harness.transform(r#"{"type":"speechStart","turnId":123456789012345678901234567890}"#);
    assert_eq!(turn_item(&large[0]), "123456789012345678901234567890");
    let negative = harness.transform(r#"{"type":"speechStart","turnId":-12}"#);
    assert_eq!(turn_item(&negative[0]), "-12");
}

#[test]
fn invalid_turn_ids_are_errors() {
    let mut harness = Harness::default();
    let invalid_turn_ids = [
        r#"{"type":"speechStart","turnId":true}"#,
        r#"{"type":"speechStart","turnId":""}"#,
        r#"{"type":"speechStart","turnId":"   "}"#,
        r#"{"type":"speechStart","turnId":1.5}"#,
        r#"{"type":"speechStart","turnId":{"a":1}}"#,
        r#"{"type":"speechEnd"}"#,
    ];
    let turn_errors: Vec<MuseProtocolError> = invalid_turn_ids
        .iter()
        .map(|payload| harness.try_transform(payload).unwrap_err())
        .collect();
    assert!(
        turn_errors
            .iter()
            .all(|error| matches!(error, MuseProtocolError::InvalidTurnId { .. }))
    );
    assert_eq!(
        turn_errors[0].to_string(),
        "speechStart event has invalid turnId"
    );
    assert_eq!(
        turn_errors[5].to_string(),
        "speechEnd event has invalid turnId"
    );
    let numeric = harness.transform(r#"{"type":"speechStart","turnId":7}"#);
    assert_eq!(turn_item(&numeric[0]), "7");
    let padded = harness.transform(r#"{"type":"speechStart","turnId":" t2 "}"#);
    assert_eq!(turn_item(&padded[0]), "t2");
}

#[test]
fn invalid_transcripts_are_errors() {
    let mut harness = Harness::default();
    let bad_transcript = harness
        .try_transform(r#"{"type":"transcript","turnId":"t2","transcript":5}"#)
        .unwrap_err();
    assert_eq!(
        bad_transcript.to_string(),
        "transcript event has invalid transcript"
    );
    let bad_complete = harness
        .try_transform(r#"{"type":"speechComplete","turnId":"t2","transcript":[]}"#)
        .unwrap_err();
    assert_eq!(
        bad_complete.to_string(),
        "speechComplete event has invalid transcript"
    );
}

#[test]
fn turns_serialize_to_the_openai_wire_shape() {
    let mut harness = Harness::default();
    let events = harness.rendered(r#"{"type":"speechComplete","turnId":"t1","transcript":"done"}"#);
    assert_eq!(
        events,
        vec![
            r#"{"type":"input_audio_buffer.speech_started","event_id":"event_1","item_id":"t1"}"#
                .to_owned(),
            r#"{"type":"input_audio_buffer.speech_stopped","event_id":"event_2","item_id":"t1"}"#
                .to_owned(),
            r#"{"type":"conversation.item.input_audio_transcription.completed","event_id":"event_3","item_id":"t1","content_index":0,"transcript":"done"}"#
                .to_owned(),
        ]
    );
    let mut delta_harness = Harness::default();
    drop(delta_harness.transform(r#"{"type":"speechStart","turnId":"t1"}"#));
    let delta = delta_harness.rendered(r#"{"type":"transcript","turnId":"t1","transcript":"hi"}"#);
    assert_eq!(
        delta,
        vec![
            r#"{"type":"conversation.item.input_audio_transcription.delta","event_id":"event_2","item_id":"t1","content_index":0,"delta":"hi"}"#
                .to_owned()
        ]
    );
}

#[test]
fn completed_events_carry_the_unbilled_duration() {
    let mut harness = Harness::default();
    drop(harness.transform(r#"{"type":"audioProgress","audioProcessedMs":1000}"#));
    let events = harness.rendered(r#"{"type":"speechComplete","turnId":"t1","transcript":"done"}"#);
    assert_eq!(
        events[2],
        r#"{"type":"conversation.item.input_audio_transcription.completed","event_id":"event_3","item_id":"t1","content_index":0,"transcript":"done","usage":{"type":"duration","seconds":1.0}}"#
    );
    assert!(harness.transformer.take_unbilled_usage().is_none());
}

#[rstest]
fn replays_the_muse_frames_corpus(#[files("fuzz/corpus/muse_frames/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let Some((&selector, rest)) = bytes.split_first() else {
        panic!("every corpus file starts with a selector byte");
    };
    let mode = match selector {
        0 => MuseMode::Endpointing,
        1 => MuseMode::PushToTalk,
        other => panic!("unknown selector {other}"),
    };
    let Ok(frame) = json_object::<MuseProtocolError>(rest) else {
        return;
    };
    let mut transformer = MuseEventTransformer::default();
    let mut ids = UuidEventIds;
    drop(transformer.transform(&frame, mode, &mut ids));
    drop(transformer.take_unbilled_usage());
}
