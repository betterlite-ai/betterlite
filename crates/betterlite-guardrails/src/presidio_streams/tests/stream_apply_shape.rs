#![cfg(test)]

use super::shared::{
    DELTA_FRAME, Harness, Kind, START_FRAME, bytes, capture_logs, chat, describe, logged_text, ok,
    request_data,
};

#[tokio::test]
async fn an_anthropic_first_frame_switches_to_raw_sse_masking() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![
        bytes(": ka\n\n"),
        bytes(START_FRAME),
        bytes(DELTA_FRAME),
    ]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, ["bytes : ka\n\n", "bytes rebuilt:MASKED"]);
    assert!(harness.assembly.seen_messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn leading_frames_without_data_go_out_before_chat_chunks_are_masked() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![bytes(": ka\n\n"), chat("Alice")]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent.first().map(String::as_str), Some("bytes : ka\n\n"));
    assert!(sent[1].contains(r#""content":"MASKED""#));
}

#[tokio::test]
async fn a_raw_stream_that_is_not_anthropic_passes_through_with_a_warning() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![
        bytes("data: {\"candidates\":[]}\n\n"),
        bytes("data: more\n\n"),
    ]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            "bytes data: {\"candidates\":[]}\n\n",
            "bytes data: more\n\n"
        ]
    );
    assert!(logged_text(&captured).contains("raw non-Anthropic SSE passthrough"));
}

#[tokio::test]
async fn an_anthropic_frame_after_a_passthrough_frame_is_not_masked() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![bytes("data: {}\n\n"), bytes(START_FRAME)]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            "bytes data: {}\n\n".to_owned(),
            format!("bytes {START_FRAME}")
        ]
    );
}

#[tokio::test]
async fn an_anthropic_frame_after_chat_chunks_is_relayed_and_the_chat_is_masked() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![chat("Alice"), bytes(START_FRAME)]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent.first(), Some(&format!("bytes {START_FRAME}")));
    assert!(sent[1].contains(r#""content":"MASKED""#));
    assert_ne!(sent[1], describe(&chat("Alice")));
}
