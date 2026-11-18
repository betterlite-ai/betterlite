use super::support;
use crate::realtime_events::decode::{
    DecodedEvent, MAX_MESSAGE_BYTES, NativeEvent, decode_realtime_client_preface,
    decode_realtime_native_event,
};
use crate::realtime_events::dispatch::{MAX_DELTA_BYTES, MAX_DELTA_CHUNKS};
use crate::realtime_events::error::EventError;
use betterlite_models::decisions::Json;

const TEXT: &str = r#"{"event":{"textOutput":{"content":"é"}}}"#;

pub(super) fn delta_accumulator_limits() {
    assert_eq!(MAX_DELTA_CHUNKS, 4096);
    assert_eq!(MAX_DELTA_BYTES, 4_194_304);
    repeated_small_frames();
    byte_boundary();
    oversized_state();
}

fn repeated_small_frames() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input_with_ids("item", "resp", "conv");
    for count in 1..=MAX_DELTA_CHUNKS {
        let response = support::transform(&mut session, &ids, TEXT, &input).expect("within cap");
        assert_eq!(
            response.current_delta_chunks.as_ref().map(Vec::len),
            Some(count)
        );
        input.current_delta_chunks = response.current_delta_chunks;
    }
    assert!(matches!(
        support::transform(&mut session, &ids, TEXT, &input),
        Err(EventError::ExceedsLimit {
            subject: "response delta chunks",
            limit: MAX_DELTA_CHUNKS
        })
    ));
    assert_eq!(
        input.current_delta_chunks.as_ref().map(Vec::len),
        Some(MAX_DELTA_CHUNKS)
    );
}

fn byte_boundary() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input_with_ids("item", "resp", "conv");
    let chunk = format!(r#"{{"delta":"{}"}}"#, "x".repeat(MAX_DELTA_BYTES - 4));
    input.current_delta_chunks = Some(vec![support::json(&chunk)]);
    for count in 2..=3 {
        let response =
            support::transform(&mut session, &ids, TEXT, &input).expect("within byte cap");
        assert_eq!(
            response.current_delta_chunks.as_ref().map(Vec::len),
            Some(count)
        );
        input.current_delta_chunks = response.current_delta_chunks;
    }
    assert!(matches!(
        support::transform(
            &mut session,
            &ids,
            r#"{"event":{"textOutput":{"content":"a"}}}"#,
            &input
        ),
        Err(EventError::ExceedsLimit {
            subject: "response delta bytes",
            limit: MAX_DELTA_BYTES
        })
    ));
    assert_eq!(input.current_delta_chunks.as_ref().map(Vec::len), Some(3));
}

fn oversized_state() {
    let mut input = support::input_with_ids("item", "resp", "conv");
    input.current_delta_chunks = Some(vec![support::json(r#"{"delta":""}"#); MAX_DELTA_CHUNKS + 1]);
    assert!(matches!(
        support::transform(
            &mut support::new_session(),
            &support::CountingIds::new(),
            "{}",
            &input
        ),
        Err(EventError::ExceedsLimit {
            subject: "response delta chunks",
            limit: MAX_DELTA_CHUNKS
        })
    ));
    let chunk = format!(r#"{{"delta":"{}"}}"#, "x".repeat(MAX_DELTA_BYTES + 1));
    input.current_delta_chunks = Some(vec![support::json(&chunk)]);
    assert!(matches!(
        support::transform(
            &mut support::new_session(),
            &support::CountingIds::new(),
            "{}",
            &input
        ),
        Err(EventError::ExceedsLimit {
            subject: "response delta bytes",
            limit: MAX_DELTA_BYTES
        })
    ));
}

// A JSON object of exactly `size` bytes, padded inside its last string field
fn padded_message(size: usize, head: &str) -> String {
    let tail = r#""}"#;
    let padding = size.saturating_sub(head.len()).saturating_sub(tail.len());
    format!("{head}{}{tail}", "x".repeat(padding))
}

// Both decoders refuse a message past the cap before parsing it, and parse one of exactly the cap
pub(super) fn decoder_limits() {
    assert_eq!(MAX_MESSAGE_BYTES, 1_048_576);
    let at_cap = padded_message(MAX_MESSAGE_BYTES, r#"{"event":{"sessionStart":{}},"pad":""#);
    assert!(matches!(
        decode_realtime_native_event(at_cap.as_bytes()),
        Ok(DecodedEvent::Event(NativeEvent::SessionStart))
    ));
    let over_cap = padded_message(
        MAX_MESSAGE_BYTES + 1,
        r#"{"event":{"sessionStart":{}},"pad":""#,
    );
    assert!(matches!(
        decode_realtime_native_event(over_cap.as_bytes()),
        Err(EventError::ExceedsLimit { .. })
    ));
    let preface = padded_message(MAX_MESSAGE_BYTES, r#"{"type":"session.update","pad":""#);
    assert_eq!(
        decode_realtime_client_preface(preface.as_bytes()).get("type"),
        Some(&Json::Text("session.update".to_owned()))
    );
    let over = padded_message(MAX_MESSAGE_BYTES + 1, r#"{"type":"session.update","pad":""#);
    assert!(decode_realtime_client_preface(over.as_bytes()).is_empty());
}

// A message that does not parse is a warning whatever its first token promises
pub(super) fn syntax_before_shape() {
    let (log, _guard) = support::capture();
    for message in ["[", "null garbage", "[] garbage", "[] {}"] {
        let response = support::transform(
            &mut support::new_session(),
            &support::CountingIds::new(),
            message,
            &support::input(),
        )
        .expect("a syntax error is a warning");
        assert_eq!(support::events(&response).len(), 0);
        assert_eq!(response.current_output_item_id, None);
        assert_eq!(response.current_delta_chunks, None);
    }
    assert!(log.text().contains("Invalid JSON message: ["));
    assert!(log.text().contains("Invalid JSON message: null garbage"));
}
