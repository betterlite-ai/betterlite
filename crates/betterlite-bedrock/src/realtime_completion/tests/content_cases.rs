use uuid::Uuid;

use super::support::{EVENT_1, FixedIds, ITEM_DONE_BODY, done_wire, open_state, wire};
use crate::agentcore_stream::IdSource as _;
use crate::realtime_completion::content_end::content_end_events;
use crate::realtime_completion::records::{DeltaChunk, DeltaType, ResponseState};
use crate::realtime_completion::usage::response_done_events;
use crate::realtime_session::RealtimeSession;

pub(super) fn content_end_text_audio() {
    let mut text = open_state(Some(DeltaType::Text), Some(&["Good ", "day"]));
    let text_events = content_end_events(&mut text, &FixedIds::sequence(3));
    assert_eq!(
        wire(&text_events),
        done_wire(
            "response.text.done",
            r#","text":"Good day""#,
            r#"{"type":"text","text":"Good day"}"#
        )
    );
    assert_eq!(text.delta_chunks, None);

    let mut audio = open_state(Some(DeltaType::Audio), Some(&["ignored"]));
    let audio_events = content_end_events(&mut audio, &FixedIds::sequence(3));
    assert_eq!(
        wire(&audio_events),
        done_wire(
            "response.audio.done",
            "",
            r#"{"type":"audio","transcript":""}"#
        )
    );
    assert_eq!(audio.delta_chunks, None);

    let mut no_chunks = open_state(Some(DeltaType::Text), None);
    let empty_events = content_end_events(&mut no_chunks, &FixedIds::sequence(3));
    assert_eq!(
        wire(&empty_events),
        done_wire(
            "response.text.done",
            r#","text":"""#,
            r#"{"type":"text","text":""}"#
        )
    );

    let mut unknown = open_state(None, Some(&["kept"]));
    let unknown_events = content_end_events(&mut unknown, &FixedIds::sequence(3));
    assert_eq!(
        wire(&unknown_events),
        [format!(
            r#"{{"type":"response.output_item.done","event_id":"{EVENT_1}",{ITEM_DONE_BODY}"#
        )]
    );
    assert_eq!(unknown.delta_chunks, None);
    missing_ids_keep_chunks();
}

fn missing_ids_keep_chunks() {
    let kept = Some(vec![DeltaChunk {
        delta: "kept".to_owned(),
    }]);
    let mut without_item = open_state(Some(DeltaType::Text), Some(&["kept"]));
    without_item.output_item_id = None;
    let ids = FixedIds::sequence(3);
    assert!(content_end_events(&mut without_item, &ids).is_empty());
    assert_eq!(without_item.delta_chunks, kept);
    let mut without_response = open_state(Some(DeltaType::Text), Some(&["kept"]));
    without_response.response_id = None;
    assert!(content_end_events(&mut without_response, &ids).is_empty());
    assert_eq!(without_response.delta_chunks, kept);
    assert_eq!(ids.next_id(), Uuid::from_u128(1));
}

pub(super) fn output_item_completion() {
    let mut state = open_state(None, None);
    let events = content_end_events(&mut state, &FixedIds::sequence(1));
    assert_eq!(
        wire(&events),
        [format!(
            r#"{{"type":"response.output_item.done","event_id":"{EVENT_1}",{ITEM_DONE_BODY}"#
        )]
    );
    assert_eq!(state.delta_chunks, None);
    assert_eq!(state.delta_type, None);
    assert_eq!(state.output_item_id.as_deref(), Some("item_1"));
    assert_eq!(state.response_id.as_deref(), Some("resp_1"));
    assert_eq!(state.conversation_id.as_deref(), Some("conv_1"));
}

pub(super) fn multiple_content_blocks() {
    let ids = FixedIds::sequence(11);
    let mut state = open_state(Some(DeltaType::Text), Some(&["one", "two"]));
    let first = wire(&content_end_events(&mut state, &ids));
    assert!(first[0].ends_with(r#""response_id":"resp_1","text":"onetwo"}"#));
    assert_eq!(state.delta_chunks, None);
    assert_eq!(state.response_id.as_deref(), Some("resp_1"));
    assert_eq!(state.output_item_id.as_deref(), Some("item_1"));

    state.delta_type = Some(DeltaType::Audio);
    let audio = wire(&content_end_events(&mut state, &ids));
    assert_eq!(audio.len(), 3);
    assert!(audio[0].contains(r#""type":"response.audio.done""#));
    assert!(audio[0].contains("event_00000000-0000-0000-0000-000000000004"));
    assert!(audio[2].contains("event_00000000-0000-0000-0000-000000000006"));

    state.delta_type = Some(DeltaType::Text);
    let again = wire(&content_end_events(&mut state, &ids));
    assert!(again[0].ends_with(r#""response_id":"resp_1","text":""}"#));
    finish_response(&mut state, &ids);
}

fn finish_response(state: &mut ResponseState, ids: &FixedIds) {
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    let done = response_done_events(&mut session, state, ids).expect("the usage counts subtract");
    assert_eq!(done.len(), 1);
    assert_eq!(
        *state,
        ResponseState {
            output_item_id: None,
            response_id: None,
            conversation_id: Some("conv_1".to_owned()),
            delta_type: None,
            delta_chunks: None,
        }
    );
    let repeated =
        response_done_events(&mut session, state, ids).expect("the usage counts subtract");
    assert!(repeated.is_empty());
    assert!(content_end_events(state, ids).is_empty());
    assert_eq!(ids.next_id(), Uuid::from_u128(11));
}
