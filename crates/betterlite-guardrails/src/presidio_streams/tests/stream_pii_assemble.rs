#![cfg(test)]

use super::shared::{Builder, Harness, Kind, chat, describe, ok, other, tokens_request};

#[tokio::test]
async fn only_chat_chunks_are_assembled_into_one_chunk() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let events = ok(vec![chat("a"), chat("b"), chat("c")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains(r#""content":"abc""#));
}

#[tokio::test]
async fn an_empty_stream_sends_nothing_and_never_calls_the_builder() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let (sent, result) = harness.run(Kind::Unmask, &mut request, Vec::new()).await;
    result.unwrap();
    assert!(sent.is_empty());
    assert!(harness.assembly.seen_messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_stream_of_only_a_non_chat_event_is_not_assembled() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Unmask, &mut request, ok(vec![other("{}")]))
        .await;
    result.unwrap();
    assert_eq!(sent, [describe(&other("{}"))]);
    assert!(harness.assembly.seen_messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_build_that_is_not_a_model_response_sends_the_chunks_as_they_came() {
    let mut harness = Harness::new();
    harness.assembly.builder = Builder::NotAModel;
    let mut request = tokens_request();
    let events = ok(vec![chat("a <PERSON_1>"), chat("b")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [describe(&chat("a <PERSON_1>")), describe(&chat("b"))]
    );
}
