use betterlite_core::api::realtime_contracts::events::AllDeltaType;

use super::support;

pub(super) fn native_session_and_state() {
    session_start_marker();
    state_preserved();
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
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"sessionStart":{}}}"#,
        &support::input(),
    )
    .expect("a session start");
    assert_eq!(
        response.session_configuration_request.as_deref(),
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
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentStart":{"role":"USER","type":"TEXT"}}}"#,
        &input,
    )
    .expect("a user content start");
    assert_eq!(support::events(&response).len(), 0);
    assert!(session.transcript.active);
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"textOutput":{"content":"hello"}}}"#,
        &input,
    )
    .expect("a transcript delta");
    assert_eq!(
        support::event_type(&support::events(&response)[0]),
        Some("conversation.item.input_audio_transcription.delta")
    );
    assert_eq!(session.transcript.buffer, "hello");
    let response = support::transform(&mut session, &ids, r#"{"event":{"textOutput":{}}}"#, &input)
        .expect("an empty transcript delta");
    assert_eq!(
        support::event_type(&support::events(&response)[0]),
        Some("conversation.item.input_audio_transcription.delta")
    );
    assert_eq!(session.transcript.buffer, "hello");
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#,
        &input,
    )
    .expect("a transcript end");
    assert_eq!(support::events(&response).len(), 1);
    assert_eq!(
        support::event_type(&support::events(&response)[0]),
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
    let events = support::transform_events(
        r#"{"event":{"textOutput":{"content":"assistant"}}}"#,
        &input,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(support::event_type(&events[0]), Some("response.text.delta"));
    let events = support::transform_events(
        r#"{"event":{"textOutput":{"content":"assistant"}}}"#,
        &support::input(),
    );
    assert_eq!(events.len(), 0);
}
