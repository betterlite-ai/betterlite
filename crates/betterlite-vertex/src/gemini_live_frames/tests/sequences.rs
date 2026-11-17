#![cfg(test)]

use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use rstest::rstest;

use super::support::{
    LIVE_MODEL, Live, TRANSCRIBE_MODEL, draw, field, json, list, lookup, number, object, text,
    types,
};
use crate::gemini_live_frames::state::CallerState;

const TOOL_MEMBER: &str =
    r#""toolCall": {"functionCalls": [{"name": "lookup", "args": {"city": "Paris"}}]}"#;
const TURN_MEMBER: &str = r#""serverContent": {"turnComplete": true}"#;
const USAGE_MEMBER: &str = r#""usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5, "totalTokenCount": 15}"#;
const TOOL_ONLY: &str =
    r#"{"toolCall": {"functionCalls": [{"name": "lookup", "args": {"city": "Paris"}}]}}"#;
const MODEL_TURN: &str = r#"{"serverContent": {"modelTurn": {"parts": [{"text": "hi"}]}}}"#;

#[test]
fn standalone_usage_waits_for_the_next_response_done() {
    let mut live = Live::new(LIVE_MODEL);
    let events = live
        .frame(r#"{"usageMetadata": {"promptTokenCount": 4}}"#)
        .expect("the usage-only frame is known");
    assert!(events.is_empty());
    assert_eq!(
        live.session.pending_usage_metadata,
        Some(object(r#"{"promptTokenCount": 4}"#))
    );
    assert_eq!(live.caller, CallerState::default());
}

#[test]
fn input_transcription_runs_before_a_tool_sibling() {
    let mut live = Live::new(LIVE_MODEL);
    let source = format!(
        r#"{{"serverContent": {{"inputTranscription": {{"text": "hello"}}}}, {TOOL_MEMBER}}}"#
    );
    let events = live.frame(&source).expect("the frame is known");
    let expected = format!(
        r#"{{"type":"conversation.item.input_audio_transcription.completed","event_id":"event_{}","transcript":"hello","item_id":"item_{}","content_index":0}}"#,
        draw(1),
        draw(2)
    );
    assert_eq!(events.first(), Some(&json(&expected)));
    assert_eq!(
        types(&events),
        [
            "conversation.item.input_audio_transcription.completed",
            "response.created",
            "response.output_item.added",
            "conversation.item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.done",
        ]
    );
    assert_eq!(
        lookup(field(&events, 1, "response"), &["id"]),
        &text(format!("resp_{}", draw(4)))
    );
    assert_eq!(
        live.caller.conversation_id,
        Some(format!("conv_{}", draw(3)))
    );
    assert_eq!(live.caller.response_id, None);
    assert_eq!(live.caller.output_item_id, None);
    assert_eq!(live.caller.delta_type, None);
    let chunks = live
        .caller
        .item_chunks
        .as_deref()
        .expect("the tool item is a chunk");
    assert_eq!(types(chunks), ["response.output_item.done"]);
}

#[test]
fn output_transcription_starts_one_audio_item() {
    let mut live = Live::new(LIVE_MODEL);
    let events = live
        .frame(r#"{"serverContent": {"outputTranscription": {"text": "hi"}}}"#)
        .expect("the frame is known");
    assert_eq!(
        types(&events),
        [
            "response.created",
            "response.output_item.added",
            "conversation.item.added",
            "response.content_part.added",
            "response.output_audio_transcript.delta",
        ]
    );
    assert_eq!(
        lookup(field(&events, 3, "part"), &["type"]),
        &text("audio".to_owned())
    );
    let expected = format!(
        r#"{{"type":"response.output_audio_transcript.delta","event_id":"event_{}","transcript":"hi","item_id":"item_{}","content_index":0,"output_index":0,"response_id":"resp_{}","delta":"hi"}}"#,
        draw(8),
        draw(2),
        draw(1)
    );
    assert_eq!(events.last(), Some(&json(&expected)));
    assert_eq!(
        live.caller.output_item_id,
        Some(format!("item_{}", draw(2)))
    );
    assert_eq!(live.caller.response_id, Some(format!("resp_{}", draw(1))));
    assert_eq!(
        live.caller.conversation_id,
        Some(format!("conv_{}", draw(3)))
    );
    assert_eq!(live.caller.delta_type, None);
    let events = live
        .frame(r#"{"serverContent": {"outputTranscription": {"text": "there"}}}"#)
        .expect("the frame is known");
    assert_eq!(types(&events), ["response.output_audio_transcript.delta"]);
    assert_eq!(
        lookup(&events[0], &["item_id"]),
        &text(format!("item_{}", draw(2)))
    );
}

#[test]
fn a_transcription_frame_never_buffers_its_usage() {
    let mut live = Live::with_unbilled(TRANSCRIBE_MODEL, 96_000);
    let source = format!(
        r#"{{"serverContent": {{"inputTranscription": {{"text": "hello"}}}}, {USAGE_MEMBER}}}"#
    );
    let events = live.frame(&source).expect("the frame is known");
    let expected = format!(
        r#"{{"type":"conversation.item.input_audio_transcription.completed","event_id":"event_{}","transcript":"hello","item_id":"item_{}","content_index":0,"usage":{{"type":"tokens","input_tokens":50,"output_tokens":6,"total_tokens":56,"input_token_details":{{"text_tokens":0,"audio_tokens":50}}}}}}"#,
        draw(1),
        draw(2)
    );
    assert_eq!(events, list(&format!("[{expected}]")));
    assert_eq!(live.session.pending_usage_metadata, None);
    assert_eq!(live.session.unbilled_input_audio_bytes, 0);
}

#[test]
fn a_tool_turn_before_the_turn_close_drains_the_frame_usage() {
    let mut live = Live::new(LIVE_MODEL);
    let source = format!("{{{TOOL_MEMBER}, {TURN_MEMBER}, {USAGE_MEMBER}}}");
    let events = live.frame(&source).expect("the frame is known");
    assert_eq!(
        types(&events),
        [
            "response.created",
            "response.output_item.added",
            "conversation.item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.done",
            "response.done",
        ]
    );
    assert_eq!(
        lookup(field(&events, 6, "response"), &["usage", "total_tokens"]),
        &number(15)
    );
    assert_eq!(
        lookup(field(&events, 7, "response"), &["usage", "total_tokens"]),
        &number(0)
    );
    assert_eq!(
        live.caller.conversation_id,
        Some(format!("conv_{}", draw(1)))
    );
    assert_eq!(live.caller.response_id, None);
    assert_eq!(live.caller.output_item_id, None);
    assert_eq!(live.session.pending_usage_metadata, None);
}

#[test]
fn a_turn_close_before_the_tool_keeps_the_usage_in_the_frame() {
    let mut live = Live::new(LIVE_MODEL);
    let source = format!("{{{TURN_MEMBER}, {TOOL_MEMBER}, {USAGE_MEMBER}}}");
    let events = live.frame(&source).expect("the frame is known");
    assert_eq!(
        types(&events),
        [
            "response.done",
            "response.created",
            "response.output_item.added",
            "conversation.item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.done",
        ]
    );
    assert_eq!(
        lookup(field(&events, 0, "response"), &["id"]),
        &text(format!("resp_{}", draw(2)))
    );
    assert_eq!(
        lookup(field(&events, 1, "response"), &["id"]),
        &text(format!("resp_{}", draw(5)))
    );
    assert_eq!(
        lookup(field(&events, 0, "response"), &["usage", "total_tokens"]),
        &number(15)
    );
    assert_eq!(
        lookup(field(&events, 7, "response"), &["usage", "total_tokens"]),
        &number(15)
    );
}

#[test]
fn a_trailing_turn_close_after_a_tool_is_suppressed() {
    let mut live = Live::new(LIVE_MODEL);
    let events = live.frame(TOOL_ONLY).expect("the tool frame is known");
    assert_eq!(types(&events).last(), Some(&"response.done"));
    assert_eq!(live.caller.response_id, None);
    let source = format!("{{{TURN_MEMBER}, {USAGE_MEMBER}}}");
    let events = live.frame(&source).expect("the trailing frame is known");
    assert!(events.is_empty());
    assert_eq!(
        live.session.pending_usage_metadata,
        Some(object(
            r#"{"promptTokenCount": 10, "candidatesTokenCount": 5, "totalTokenCount": 15}"#
        ))
    );
    assert_eq!(live.caller.item_chunks.as_ref().map(Vec::len), Some(1));
}

#[test]
fn content_after_a_tool_turn_opens_a_response_and_bills_the_buffer() {
    let mut live = Live::new(LIVE_MODEL);
    live.frame(TOOL_ONLY).expect("the tool frame is known");
    let trailing = format!("{{{TURN_MEMBER}, {USAGE_MEMBER}}}");
    live.frame(&trailing)
        .expect("the suppressed close is known");
    let events = live.frame(MODEL_TURN).expect("the model turn is known");
    assert_eq!(
        types(&events),
        [
            "response.created",
            "response.output_item.added",
            "conversation.item.added",
            "response.content_part.added",
            "response.output_text.delta",
        ]
    );
    assert_eq!(live.caller.response_id, Some(format!("resp_{}", draw(12))));
    assert_eq!(
        live.caller.output_item_id,
        Some(format!("item_{}", draw(13)))
    );
    let events = live
        .frame(r#"{"serverContent": {"turnComplete": true}}"#)
        .expect("the turn close is known");
    assert_eq!(types(&events), ["response.done"]);
    assert_eq!(
        lookup(field(&events, 0, "response"), &["usage", "total_tokens"]),
        &number(15)
    );
    assert_eq!(live.session.pending_usage_metadata, None);
    assert_eq!(live.caller.response_id, None);
}

#[test]
fn unknown_siblings_are_ignored_beside_a_usage_frame() {
    let mut live = Live::new(LIVE_MODEL);
    let source = r#"{"usageMetadata": {"promptTokenCount": 2}, "sessionResumptionUpdate": {"newHandle": "h"}}"#;
    let events = live.frame(source).expect("unknown siblings are ignored");
    assert!(events.is_empty());
    assert_eq!(
        live.session.pending_usage_metadata,
        Some(object(r#"{"promptTokenCount": 2}"#))
    );
    let events = live
        .frame(r#"{"goAway": {"timeLeft": "1s"}}"#)
        .expect("the frame is known");
    assert!(events.is_empty());
    assert_eq!(live.caller, CallerState::default());
}

#[test]
fn generation_complete_without_a_delta_emits_nothing() {
    let mut live = Live::new(LIVE_MODEL);
    let events = live
        .frame(r#"{"serverContent": {"generationComplete": true}}"#)
        .expect("the frame is known");
    assert!(events.is_empty());
    assert_eq!(live.caller.delta_type, None);
    let events = live.frame(MODEL_TURN).expect("the model turn is known");
    assert_eq!(types(&events).last(), Some(&"response.output_text.delta"));
    let events = live
        .frame(r#"{"serverContent": {"generationComplete": true}}"#)
        .expect("the close is known");
    assert_eq!(
        types(&events),
        [
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
        ]
    );
    assert_eq!(live.caller.delta_type, Some(AllDeltaType::Text));
}

#[test]
fn setup_complete_reports_the_session() {
    let mut live = Live::new(LIVE_MODEL);
    let events = live
        .frame(r#"{"setupComplete": {}}"#)
        .expect("the setup frame is known");
    let expected = format!(
        r#"[{{"type":"session.created","session":{{"id":"session-log","modalities":["audio"],"model":"gemini-2.5-flash"}},"event_id":"{}"}}]"#,
        draw(1)
    );
    assert_eq!(events, list(&expected));
    assert_eq!(live.caller, CallerState::default());
}

#[rstest]
#[case::empty("{}", 0)]
#[case::setup(r#"{"setupComplete": {}}"#, 1)]
#[case::content(r#"{"serverContent": {}}"#, 0)]
#[case::tool(r#"{"toolCall": {}}"#, 0)]
fn empty_known_payloads_emit_only_what_the_source_emits(
    #[case] source: &str,
    #[case] expected: usize,
) {
    let mut live = Live::new(LIVE_MODEL);
    let events = live.frame(source).expect("the frame is known");
    assert_eq!(events.len(), expected);
    assert_eq!(live.session.pending_usage_metadata, None);
}
