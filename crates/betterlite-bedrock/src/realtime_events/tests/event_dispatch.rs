use betterlite_core::api::realtime_contracts::events::{AllDeltaType, RealtimeResponseTypedDict};
use betterlite_models::decisions::Json;

use super::support::{self, SECRET};
use crate::realtime_events::decode::{DecodedEvent, NativeEvent, decode_realtime_native_event};
use crate::realtime_events::error::EventError;

pub(super) fn native_event_dispatch() {
    decoding_errors();
    malformed_message();
    unknown_payloads();
    branch_priority();
    speech_priority();
    tool_and_end_priority();
    transcript_content_end();
}

fn decoding_errors() {
    assert!(matches!(
        decode_realtime_native_event(b"[]"),
        Err(EventError::NotAnObject { .. })
    ));
    assert!(matches!(
        decode_realtime_native_event(br#"{"event":3}"#),
        Err(EventError::EventNotAnObject { .. })
    ));
    assert!(matches!(
        decode_realtime_native_event(&[0xff]),
        Err(EventError::Utf8 { .. })
    ));
    payload_errors();
    assert!(matches!(
        decode_realtime_native_event(
            br#"{"event":{"contentStart":{"role":"USER","type":"TEXT"}}}"#
        ),
        Ok(DecodedEvent::Event(NativeEvent::ContentStart(_)))
    ));
}

fn payload_errors() {
    assert!(matches!(
        decode_realtime_native_event(br#"{"event":{"contentStart":3}}"#),
        Err(EventError::Payload {
            field: "contentStart",
            ..
        })
    ));
    assert!(matches!(
        decode_realtime_native_event(br#"{"event":{"textOutput":"x"}}"#),
        Err(EventError::Payload {
            field: "textOutput",
            ..
        })
    ));
    assert!(matches!(
        decode_realtime_native_event(br#"{"event":{"audioOutput":3}}"#),
        Err(EventError::Payload {
            field: "audioOutput",
            ..
        })
    ));
    assert!(matches!(
        decode_realtime_native_event(br#"{"event":{"usageEvent":null}}"#),
        Err(EventError::Payload {
            field: "usageEvent",
            ..
        })
    ));
    assert!(matches!(
        decode_realtime_native_event(br#"{"event":{"toolUse":[]}}"#),
        Err(EventError::Payload {
            field: "toolUse",
            ..
        })
    ));
}

fn malformed_message() {
    let (log, _guard) = support::capture();
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input_with_ids("item_old", "resp_old", "conv_old");
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"a"}"#)]);
    input.current_item_chunks = Some(vec![support::json(r#"{"delta":"b"}"#)]);
    input.current_delta_type = Some(AllDeltaType::Audio);
    input.session_configuration_request = Some(r#"{"configured": true}"#.to_owned());
    let response = support::transform(&mut session, &ids, r#"{"event": "#, &input)
        .expect("a syntax error is a warning");
    let expected = RealtimeResponseTypedDict {
        response: Json::List(Vec::new()),
        current_output_item_id: input.current_output_item_id.clone(),
        current_response_id: input.current_response_id.clone(),
        current_delta_chunks: input.current_delta_chunks.clone(),
        current_conversation_id: input.current_conversation_id.clone(),
        current_item_chunks: input.current_item_chunks.clone(),
        current_delta_type: input.current_delta_type,
        session_configuration_request: input.session_configuration_request.clone(),
    };
    assert_eq!(response, expected);
    assert!(log.text().contains(r#"Invalid JSON message: {"event": "#));
    let secret_message = format!(r#"{{"secret":"{SECRET}"#);
    let response = support::transform(&mut session, &ids, &secret_message, &input)
        .expect("a syntax error is a warning");
    assert_eq!(support::events(&response).len(), 0);
    assert!(log.text().contains("REDACTED"));
    assert!(!log.text().contains(SECRET));
    let long = format!(r#"{{"broken":{}"#, "x".repeat(250));
    support::transform(&mut session, &ids, &long, &input).expect("a syntax error is a warning");
    assert!(log.text().contains(&"x".repeat(200)));
    assert!(!log.text().contains(&"x".repeat(201)));
}

fn unknown_payloads() {
    let input = support::input_with_ids("item", "resp", "conv");
    let messages = [
        r#"{"event":{}}"#,
        r#"{}"#,
        r#"{"event":{"sessionEnd":{}}}"#,
        r#"{"event":{"unknownThing":1}}"#,
    ];
    for message in messages {
        let events = support::transform_events(message, &input);
        assert_eq!(events.len(), 0);
    }
}

fn branch_priority() {
    let input = support::input_with_ids("item", "resp", "conv");
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"sessionStart":{},"contentStart":{"role":"USER","type":"TEXT"}}}"#,
        &input,
    )
    .expect("sessionStart wins");
    assert_eq!(support::events(&response).len(), 0);
    assert!(!session.transcript.active);
    assert_eq!(
        response.session_configuration_request.as_deref(),
        Some(r#"{"configured": true}"#)
    );

    let events = support::transform_events(
        r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"},"textOutput":{"content":"x"}}}"#,
        &input,
    );
    assert_eq!(events.len(), 3);
    assert_eq!(support::event_type(&events[0]), Some("response.created"));

    let events = support::transform_events(
        r#"{"event":{"textOutput":{"content":"x"},"audioOutput":{"content":"y"}}}"#,
        &input,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(support::event_type(&events[0]), Some("response.text.delta"));
}

fn speech_priority() {
    let input = support::input_with_ids("item", "resp", "conv");
    let events = support::transform_events(
        r#"{"event":{"audioOutput":{"content":"y"},"contentEnd":{"stopReason":"END_TURN"}}}"#,
        &input,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(
        support::event_type(&events[0]),
        Some("response.audio.delta")
    );

    let events = support::transform_events(
        r#"{"event":{"userSpeechStart":{},"userSpeechEnd":{}}}"#,
        &input,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(
        support::event_type(&events[0]),
        Some("input_audio_buffer.speech_started")
    );

    let events = support::transform_events(
        r#"{"event":{"userSpeechEnd":{},"usageEvent":{"totalInputTokens":5}}}"#,
        &input,
    );
    assert_eq!(
        support::event_type(&events[0]),
        Some("input_audio_buffer.speech_stopped")
    );
}

fn tool_and_end_priority() {
    let input = support::input_with_ids("item", "resp", "conv");
    let events = support::transform_events(
        r#"{"event":{"usageEvent":{"totalInputTokens":5},"toolUse":{"toolUseId":"call","toolName":"lookup"}}}"#,
        &input,
    );
    assert_eq!(events.len(), 0);

    let events = support::transform_events(
        r#"{"event":{"toolUse":{"toolUseId":"call","toolName":"lookup"},"promptEnd":{}}}"#,
        &input,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(
        support::event_type(&events[0]),
        Some("response.function_call_arguments.done")
    );

    let events =
        support::transform_events(r#"{"event":{"promptEnd":{},"completionEnd":{}}}"#, &input);
    assert_eq!(events.len(), 1);
    assert_eq!(support::event_type(&events[0]), Some("response.done"));
}

fn transcript_content_end() {
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
    let response = support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"},"userSpeechStart":{}}}"#,
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

    let events = support::transform_events(
        r#"{"event":{"contentEnd":{"stopReason":"END_TURN"},"userSpeechStart":{}}}"#,
        &input,
    );
    assert_eq!(events.len(), 4);
    assert_eq!(support::event_type(&events[0]), Some("response.text.done"));
    assert_eq!(support::event_type(&events[3]), Some("response.done"));
}
