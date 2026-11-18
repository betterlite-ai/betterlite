use betterlite_core::api::realtime_contracts::events::{
    AllDeltaType, RealtimeResponseTransformInput,
};
use betterlite_models::decisions::Json;

use super::support;
use crate::realtime_events::decode::json_dict;
use crate::realtime_session::RealtimeSession;

pub(super) fn native_session_and_state() {
    session_start_marker();
    state_preserved();
    content_start_with_ids();
}

// A response id already in the state keeps response.created out and the item id in the events
fn content_start_with_ids() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let input = support::input_with_ids("item_kept", "resp_kept", "conv_kept");
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"AUDIO"}}}"#,
        &input,
    )
    .expect("an assistant content start on an open response");
    let events = support::events(&response);
    assert_eq!(events.len(), 2);
    assert_eq!(
        support::event_type(&events[0]),
        Some("response.output_item.added")
    );
    assert_eq!(
        support::event_type(&events[1]),
        Some("response.content_part.added")
    );
    let item = json_dict(&events[0])
        .and_then(|fields| fields.get("item"))
        .and_then(json_dict)
        .expect("the added item");
    assert_eq!(item.get("id"), Some(&Json::Text("item_kept".to_owned())));
    assert_eq!(
        response.current_output_item_id.as_deref(),
        Some("item_kept")
    );
    assert_eq!(response.current_response_id.as_deref(), Some("resp_kept"));
    assert_eq!(
        response.current_conversation_id.as_deref(),
        Some("conv_kept")
    );
    assert_eq!(response.current_delta_type, Some(AllDeltaType::Audio));
}

fn session_start_marker() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input_with_ids("item", "resp", "conv");
    input.session_configuration_request = Some("old".to_owned());
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"a"}"#)]);
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"sessionStart":{}}}"#,
        &input,
    )
    .expect("a session start");
    assert_eq!(support::events(&response).len(), 0);
    assert_eq!(
        response.session_configuration_request.as_deref(),
        Some(r#"{"configured": true}"#)
    );
    assert_eq!(response.current_output_item_id.as_deref(), Some("item"));
    assert_eq!(response.current_response_id.as_deref(), Some("resp"));
    assert_eq!(response.current_conversation_id.as_deref(), Some("conv"));
    assert_eq!(response.current_delta_chunks, input.current_delta_chunks);
    let fresh = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"sessionStart":{}}}"#,
        &support::input(),
    )
    .expect("a session start");
    assert_eq!(
        fresh.session_configuration_request.as_deref(),
        Some(r#"{"configured": true}"#)
    );
}

fn state_preserved() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input_with_ids("item", "resp", "conv");
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"a"}"#)]);
    input.current_item_chunks = Some(vec![support::json(r#"{"delta":"b"}"#)]);
    input.current_delta_type = Some(AllDeltaType::Audio);
    input.session_configuration_request = Some("configured".to_owned());
    let messages = [r#"{"event":{}}"#, "{}", r#"{"event":{"sessionEnd":{}}}"#];
    for message in messages {
        let response =
            support::transform(&mut session, &ids, message, &input).expect("a silent event");
        assert_eq!(support::events(&response).len(), 0);
        assert_eq!(
            response.current_output_item_id,
            input.current_output_item_id
        );
        assert_eq!(response.current_response_id, input.current_response_id);
        assert_eq!(
            response.current_conversation_id,
            input.current_conversation_id
        );
        assert_eq!(response.current_delta_chunks, input.current_delta_chunks);
        assert_eq!(response.current_item_chunks, input.current_item_chunks);
        assert_eq!(response.current_delta_type, input.current_delta_type);
        assert_eq!(
            response.session_configuration_request,
            input.session_configuration_request
        );
    }
}

pub(super) fn native_user_transcript_dispatch() {
    user_transcript_deltas();
    speculative_transcript();
    inactive_text_output();
}

fn user_transcript_deltas() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let input = support::input_with_ids("item", "resp", "conv");
    let started = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentStart":{"role":"USER","type":"TEXT"}}}"#,
        &input,
    )
    .expect("a user content start");
    assert_eq!(support::events(&started).len(), 0);
    assert!(session.transcript.active);
    let hello = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"textOutput":{"content":"hello"}}}"#,
        &input,
    )
    .expect("a transcript delta");
    assert_eq!(
        support::event_type(&support::events(&hello)[0]),
        Some("conversation.item.input_audio_transcription.delta")
    );
    assert_eq!(session.transcript.buffer, "hello");
    let empty = support::transform(&mut session, &ids, r#"{"event":{"textOutput":{}}}"#, &input)
        .expect("an empty transcript delta");
    assert_eq!(
        support::event_type(&support::events(&empty)[0]),
        Some("conversation.item.input_audio_transcription.delta")
    );
    assert_eq!(session.transcript.buffer, "hello");
    let transcript_id = session.transcript.item_id.clone();
    let stage = session.transcript.generation_stage.clone();
    transcript_audio(&mut session, &ids, &input);
    assert!(session.transcript.active);
    assert_eq!(session.transcript.buffer, "hello");
    assert_eq!(session.transcript.item_id, transcript_id);
    assert_eq!(session.transcript.generation_stage, stage);
    transcript_end(&mut session, &ids, &input);
}

// An audioOutput during an active transcription still reaches the client as an audio delta
fn transcript_audio(
    session: &mut RealtimeSession,
    ids: &support::CountingIds,
    input: &RealtimeResponseTransformInput,
) {
    let audio = support::transform(
        session,
        ids,
        r#"{"event":{"audioOutput":{"content":"YXVkaW8="}}}"#,
        input,
    )
    .expect("an audio delta during a transcript");
    let events = support::events(&audio);
    assert_eq!(events.len(), 1);
    assert_eq!(
        support::event_type(&events[0]),
        Some("response.audio.delta")
    );
    let delta = json_dict(&events[0]).expect("an audio delta object");
    assert_eq!(delta.get("delta"), Some(&Json::Text("YXVkaW8=".to_owned())));
    assert_eq!(delta.get("item_id"), Some(&Json::Text("item".to_owned())));
    assert_eq!(
        delta.get("response_id"),
        Some(&Json::Text("resp".to_owned()))
    );
    assert_eq!(audio.current_delta_chunks, input.current_delta_chunks);
}

fn transcript_end(
    session: &mut RealtimeSession,
    ids: &support::CountingIds,
    input: &RealtimeResponseTransformInput,
) {
    let ended = support::transform(
        session,
        ids,
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#,
        input,
    )
    .expect("a transcript end");
    assert_eq!(support::events(&ended).len(), 1);
    assert_eq!(
        support::event_type(&support::events(&ended)[0]),
        Some("conversation.item.input_audio_transcription.completed")
    );
    assert!(!session.transcript.active);
    assert!(session.transcript.buffer.is_empty());
}

fn speculative_transcript() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let input = support::input_with_ids("item", "resp", "conv");
    let start = r#"{"event":{"contentStart":{"role":"USER","type":"TEXT","additionalModelFields":"{\"generationStage\":\"SPECULATIVE\"}"}}}"#;
    support::transform(&mut session, &ids, start, &input).expect("a speculative content start");
    assert_eq!(
        session.transcript.generation_stage.as_deref(),
        Some("SPECULATIVE")
    );
    support::transform(
        &mut session,
        &ids,
        r#"{"event":{"textOutput":{"content":"spec"}}}"#,
        &input,
    )
    .expect("a speculative delta");
    assert!(session.transcript.buffer.is_empty());
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#,
        &input,
    )
    .expect("a transcript end");
    assert_eq!(support::events(&response).len(), 0);
    assert!(!session.transcript.active);
    assert_eq!(session.transcript.generation_stage, None);
}

fn inactive_text_output() {
    let input = support::input_with_ids("item", "resp", "conv");
    let with_ids = support::transform_events(
        r#"{"event":{"textOutput":{"content":"assistant"}}}"#,
        &input,
    );
    assert_eq!(with_ids.len(), 1);
    assert_eq!(
        support::event_type(&with_ids[0]),
        Some("response.text.delta")
    );
    let without_ids = support::transform_events(
        r#"{"event":{"textOutput":{"content":"assistant"}}}"#,
        &support::input(),
    );
    assert_eq!(without_ids.len(), 0);
}

pub(super) fn transcript_content_end() {
    let input = support::input_with_ids("item", "resp", "conv");
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentStart":{"role":"USER","type":"TEXT"}}}"#,
        &input,
    )
    .expect("a user content start");
    support::transform(
        &mut session,
        &ids,
        r#"{"event":{"textOutput":{"content":"hello"}}}"#,
        &input,
    )
    .expect("a transcript delta");
    let ended = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"},"userSpeechStart":{}}}"#,
        &input,
    )
    .expect("a transcript end");
    assert_eq!(support::events(&ended).len(), 1);
    assert_eq!(
        support::event_type(&support::events(&ended)[0]),
        Some("conversation.item.input_audio_transcription.completed")
    );
    assert!(!session.transcript.active);
    assert!(session.transcript.buffer.is_empty());
    closed_content_end(&input);
}

fn closed_content_end(input: &RealtimeResponseTransformInput) {
    let events = support::transform_events(
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"},"userSpeechStart":{}}}"#,
        input,
    );
    assert_eq!(events.len(), 2);
    assert_eq!(
        support::event_type(&events[0]),
        Some("response.output_item.done")
    );
    assert_eq!(support::event_type(&events[1]), Some("response.done"));
}
