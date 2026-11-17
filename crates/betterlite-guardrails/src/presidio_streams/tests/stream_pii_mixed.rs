#![cfg(test)]

use super::shared::{Harness, Kind, bytes, chat, describe, ok, other, tokens_request};

const EVENT: &str = r#"{"type":"response.created"}"#;

#[tokio::test]
async fn a_non_chat_event_flushes_the_buffered_chunks_before_it() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let events = ok(vec![
        chat("a <PERSON_1>"),
        chat("b"),
        other(EVENT),
        chat("c"),
    ]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            describe(&chat("a <PERSON_1>")),
            describe(&chat("b")),
            describe(&other(EVENT)),
            describe(&chat("c")),
        ]
    );
    assert!(harness.assembly.seen_messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_bytes_chunk_goes_out_at_once_while_chat_chunks_keep_buffering() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let events = ok(vec![chat("a"), bytes("data: x\n\n"), chat("b")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent.first().map(String::as_str), Some("bytes data: x\n\n"));
    assert!(sent[1].contains(r#""content":"ab""#));
}
