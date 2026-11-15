use uuid::Uuid;

use super::support::{
    EVENT_1, EVENT_2, EVENT_3, EVENT_4, EVENT_5, EVENT_6, FixedIds, ITEM_DONE_BODY, open_state,
    wire,
};
use crate::agentcore_stream::IdSource;
use crate::realtime_completion::records::{DeltaChunk, DeltaType, ResponseState};
use crate::realtime_session::RealtimeSession;

pub(super) fn content_end_text_audio() {
    let mut text = open_state(Some(DeltaType::Text), Some(&["Hel", "lo"]));
    let events = text.content_end_events(&FixedIds::sequence(3));
    assert_eq!(
        wire(&events),
        [
            format!(
                r#"{{"type":"response.text.done","content_index":0,"event_id":"{EVENT_1}","item_id":"item_1","output_index":0,"response_id":"resp_1","text":"Hello"}}"#
            ),
            format!(
                r#"{{"type":"response.content_part.done","content_index":0,"event_id":"{EVENT_2}","item_id":"item_1","output_index":0,"part":{{"type":"text","text":"Hello"}},"response_id":"resp_1"}}"#
            ),
            format!(
                r#"{{"type":"response.output_item.done","event_id":"{EVENT_3}",{ITEM_DONE_BODY}"#
            ),
        ]
    );
    assert_eq!(text.delta_chunks, None);

    let mut audio = open_state(Some(DeltaType::Audio), Some(&["ignored"]));
    let events = audio.content_end_events(&FixedIds::sequence(3));
    assert_eq!(
        wire(&events),
        [
            format!(
                r#"{{"type":"response.audio.done","content_index":0,"event_id":"{EVENT_1}","item_id":"item_1","output_index":0,"response_id":"resp_1"}}"#
            ),
            format!(
                r#"{{"type":"response.content_part.done","content_index":0,"event_id":"{EVENT_2}","item_id":"item_1","output_index":0,"part":{{"type":"audio","transcript":""}},"response_id":"resp_1"}}"#
            ),
            format!(
                r#"{{"type":"response.output_item.done","event_id":"{EVENT_3}",{ITEM_DONE_BODY}"#
            ),
        ]
    );
    assert_eq!(audio.delta_chunks, None);

    let mut no_chunks = open_state(Some(DeltaType::Text), None);
    let events = no_chunks.content_end_events(&FixedIds::sequence(3));
    let wired = wire(&events);
    assert!(wired[0].ends_with(r#""response_id":"resp_1","text":""}"#));
    assert!(wired[1].contains(r#""part":{"type":"text","text":""}"#));

    let mut unknown = open_state(None, Some(&["kept"]));
    let events = unknown.content_end_events(&FixedIds::sequence(3));
    assert_eq!(
        wire(&events),
        [format!(
            r#"{{"type":"response.output_item.done","event_id":"{EVENT_1}",{ITEM_DONE_BODY}"#
        )]
    );
    assert_eq!(unknown.delta_chunks, None);
    missing_ids_keep_chunks();
}

fn missing_ids_keep_chunks() {
    let mut without_item = open_state(Some(DeltaType::Text), Some(&["kept"]));
    without_item.output_item_id = None;
    let ids = FixedIds::sequence(3);
    assert!(without_item.content_end_events(&ids).is_empty());
    assert_eq!(
        without_item.delta_chunks,
        Some(vec![DeltaChunk {
            delta: "kept".to_owned()
        }])
    );
    let mut without_response = open_state(Some(DeltaType::Text), Some(&["kept"]));
    without_response.response_id = None;
    assert!(without_response.content_end_events(&ids).is_empty());
    assert_eq!(
        without_response.delta_chunks,
        Some(vec![DeltaChunk {
            delta: "kept".to_owned()
        }])
    );
    assert_eq!(ids.next_id(), Uuid::from_u128(1));
}

pub(super) fn output_item_completion() {
    let mut state = open_state(None, None);
    let events = state.content_end_events(&FixedIds::sequence(1));
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
    let ids = FixedIds::sequence(8);
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    let mut state = open_state(Some(DeltaType::Text), Some(&["one"]));
    let text_events = state.content_end_events(&ids);
    assert_eq!(text_events.len(), 3);
    assert_eq!(state.delta_chunks, None);
    assert_eq!(state.response_id.as_deref(), Some("resp_1"));
    assert_eq!(state.output_item_id.as_deref(), Some("item_1"));

    state.delta_type = Some(DeltaType::Audio);
    let audio_events = state.content_end_events(&ids);
    assert_eq!(
        wire(&audio_events),
        [
            format!(
                r#"{{"type":"response.audio.done","content_index":0,"event_id":"{EVENT_4}","item_id":"item_1","output_index":0,"response_id":"resp_1"}}"#
            ),
            format!(
                r#"{{"type":"response.content_part.done","content_index":0,"event_id":"{EVENT_5}","item_id":"item_1","output_index":0,"part":{{"type":"audio","transcript":""}},"response_id":"resp_1"}}"#
            ),
            format!(
                r#"{{"type":"response.output_item.done","event_id":"{EVENT_6}",{ITEM_DONE_BODY}"#
            ),
        ]
    );

    let done = session
        .response_done_events(&mut state, &ids)
        .expect("the usage counts subtract");
    assert_eq!(done.len(), 1);
    assert_eq!(
        state,
        ResponseState {
            output_item_id: None,
            response_id: None,
            conversation_id: Some("conv_1".to_owned()),
            delta_type: None,
            delta_chunks: None,
        }
    );
    let again = session
        .response_done_events(&mut state, &ids)
        .expect("the usage counts subtract");
    assert!(again.is_empty());
    assert!(state.content_end_events(&ids).is_empty());
    assert_eq!(ids.next_id(), Uuid::from_u128(8));
}
