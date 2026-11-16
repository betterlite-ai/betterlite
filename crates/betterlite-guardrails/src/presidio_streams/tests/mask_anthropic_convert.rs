#![cfg(test)]

use super::shared::{
    Builder, DELTA_FRAME, Harness, Kind, START_FRAME, bytes, capture_logs, chat, describe,
    logged_text, ok, request_data,
};

#[tokio::test]
async fn unchanged_text_returns_the_original_chunks_byte_for_byte() {
    let harness = Harness::with_base("http://presidio.invalid", Some("HelloAlice"));
    let mut request = request_data("{}");
    let events = ok(vec![bytes(START_FRAME), bytes(DELTA_FRAME)]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            format!("bytes {START_FRAME}"),
            format!("bytes {DELTA_FRAME}")
        ]
    );
}

#[tokio::test]
async fn changed_text_is_replayed_as_rebuilt_frames() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![bytes(START_FRAME), bytes(DELTA_FRAME)]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, ["bytes rebuilt:MASKED"]);
}

#[tokio::test]
async fn a_stream_that_does_not_assemble_is_returned_with_a_warning() {
    let (captured, _guard) = capture_logs();
    let mut harness = Harness::new();
    harness.assembly.builder = Builder::NotAModel;
    let mut request = request_data("{}");
    let events = ok(vec![bytes(START_FRAME), bytes(DELTA_FRAME), chat("later")]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            format!("bytes {START_FRAME}"),
            format!("bytes {DELTA_FRAME}"),
            describe(&chat("later")),
        ]
    );
    assert!(logged_text(&captured).contains("could not be assembled into a response"));
}
