#![cfg(test)]

use super::shared::{Harness, Kind, bytes, describe, ok, other, request_data, tokens_request};

const DELTA: &str =
    r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi <PERSON_1>"}}"#;
const COMPLETED: &str = r#"{"type":"response.completed","response":{"output":[{"content":[{"text":"Hi <PERSON_1>"}]}]}}"#;
const CREATED: &str =
    r#"{"type":"response.created","response":{"output":[{"content":[{"text":"<PERSON_1>"}]}]}}"#;

#[tokio::test]
async fn bytes_and_completed_events_are_unmasked_and_the_rest_passes() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let events = ok(vec![bytes(DELTA), other(COMPLETED), other(CREATED)]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            format!("bytes {}", DELTA.replace("<PERSON_1>", "Alice")),
            describe(&other(&COMPLETED.replace("<PERSON_1>", "Alice"))),
            describe(&other(CREATED)),
        ]
    );
}

#[tokio::test]
async fn without_tokens_a_raw_chunk_keeps_its_line_endings() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![bytes("data: x\r\n\r\n")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, ["bytes data: x\r\n\r\n"]);
}

#[tokio::test]
async fn without_tokens_nothing_is_unmasked() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![bytes(DELTA), other(COMPLETED)]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [format!("bytes {DELTA}"), describe(&other(COMPLETED))]
    );
}
