use betterlite_core::api::realtime_contracts::events::{AllDeltaType, RealtimeResponseTypedDict};
use betterlite_models::decisions::Json;

use super::support::{self, SECRET};
use crate::realtime_events::decode::{DecodedEvent, NativeEvent, decode_realtime_native_event};
use crate::realtime_events::error::EventError;

pub(super) fn native_event_dispatch() {
    decoding_errors();
    super::limits::decoder_limits();
    malformed_message();
    super::limits::syntax_before_shape();
    unknown_payloads();
    branch_priority();
    speech_priority();
    tool_and_end_priority();
    content_end_payload();
    super::session_cases::transcript_content_end();
}

fn content_end_payload() {
    let mut input = support::input_with_ids("item", "resp", "conv");
    input.current_delta_type = Some(AllDeltaType::Text);
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":"Hi"}"#)]);
    let error = support::transform(
        &mut support::new_session(),
        &support::CountingIds::new(),
        r#"{"event":{"contentEnd":[]}}"#,
        &input,
    )
    .expect_err("an array content end");
    assert!(matches!(
        error,
        EventError::Payload {
            field: "contentEnd",
            ..
        }
    ));
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    support::transform(
        &mut session,
        &ids,
        r#"{"event":{"contentStart":{"role":"USER","type":"TEXT"}}}"#,
        &input,
    )
    .expect("a user content start");
    let response = support::transform(&mut session, &ids, r#"{"event":{"contentEnd":[]}}"#, &input)
        .expect("a transcript end never reads the content end payload");
    assert_eq!(support::events(&response).len(), 0);
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
        decode_realtime_native_event(br#"{"event":[]}"#),
        Err(EventError::EventNotAnObject { .. })
    ));
    assert!(matches!(
        decode_realtime_native_event(br#"{"event":"x"}"#),
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
    for (message, field) in [
        (r#"{"event":{"contentStart":[]}}"#, "contentStart"),
        (r#"{"event":{"textOutput":[]}}"#, "textOutput"),
        (r#"{"event":{"audioOutput":[]}}"#, "audioOutput"),
        (r#"{"event":{"usageEvent":[]}}"#, "usageEvent"),
    ] {
        let error = decode_realtime_native_event(message.as_bytes()).expect_err("an array payload");
        assert!(matches!(error, EventError::Payload { field: actual, .. } if actual == field));
    }
    scalar_payloads();
}

fn scalar_payloads() {
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
    let redacted = support::transform(&mut session, &ids, &secret_message, &input)
        .expect("a syntax error is a warning");
    assert_eq!(support::events(&redacted).len(), 0);
    assert!(log.text().contains("REDACTED"));
    assert!(!log.text().contains(SECRET));
    let truncated = format!(r#"{{"broken":{}"#, "x".repeat(250));
    support::transform(&mut session, &ids, &truncated, &input)
        .expect("a syntax error is a warning");
    assert!(
        log.text()
            .contains(&format!(r#"{{"broken":{}"#, "x".repeat(190)))
    );
    assert!(
        !log.text()
            .contains(&format!(r#"{{"broken":{}"#, "x".repeat(191)))
    );
}

fn unknown_payloads() {
    let input = support::input_with_ids("item", "resp", "conv");
    let messages = [
        r#"{"event":{}}"#,
        "{}",
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

    let content_events = support::transform_events(
        r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"},"textOutput":{"content":"x"}}}"#,
        &support::input(),
    );
    assert_eq!(content_events.len(), 3);
    assert_eq!(
        support::event_type(&content_events[0]),
        Some("response.created")
    );

    let text_events = support::transform_events(
        r#"{"event":{"textOutput":{"content":"x"},"audioOutput":{"content":"y"}}}"#,
        &input,
    );
    assert_eq!(text_events.len(), 1);
    assert_eq!(
        support::event_type(&text_events[0]),
        Some("response.text.delta")
    );
}

fn speech_priority() {
    let input = support::input_with_ids("item", "resp", "conv");
    let audio_events = support::transform_events(
        r#"{"event":{"audioOutput":{"content":"y"},"contentEnd":{"stopReason":"END_TURN"}}}"#,
        &input,
    );
    assert_eq!(audio_events.len(), 1);
    assert_eq!(
        support::event_type(&audio_events[0]),
        Some("response.audio.delta")
    );

    let start_events = support::transform_events(
        r#"{"event":{"userSpeechStart":{},"userSpeechEnd":{}}}"#,
        &input,
    );
    assert_eq!(start_events.len(), 1);
    assert_eq!(
        support::event_type(&start_events[0]),
        Some("input_audio_buffer.speech_started")
    );

    let end_events = support::transform_events(
        r#"{"event":{"userSpeechEnd":{},"usageEvent":{"totalInputTokens":5}}}"#,
        &input,
    );
    assert_eq!(
        support::event_type(&end_events[0]),
        Some("input_audio_buffer.speech_stopped")
    );
}

fn tool_and_end_priority() {
    let input = support::input_with_ids("item", "resp", "conv");
    let usage_events = support::transform_events(
        r#"{"event":{"usageEvent":{"totalInputTokens":5},"toolUse":{"toolUseId":"call","toolName":"lookup"}}}"#,
        &input,
    );
    assert_eq!(usage_events.len(), 0);

    let tool_events = support::transform_events(
        r#"{"event":{"toolUse":{"toolUseId":"call","toolName":"lookup"},"promptEnd":{}}}"#,
        &input,
    );
    assert_eq!(tool_events.len(), 1);
    assert_eq!(
        support::event_type(&tool_events[0]),
        Some("response.function_call_arguments.done")
    );

    let end_events =
        support::transform_events(r#"{"event":{"promptEnd":{},"completionEnd":{}}}"#, &input);
    assert_eq!(end_events.len(), 1);
    assert_eq!(support::event_type(&end_events[0]), Some("response.done"));
}
