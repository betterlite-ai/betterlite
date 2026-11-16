#![cfg(test)]

use super::shared::{Harness, Kind, chat, ok, request_data};

#[tokio::test]
async fn assembles_masks_and_replays_the_chunks_as_one() {
    let harness = Harness::new();
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let (sent, result) = harness
        .run(
            Kind::Mask,
            &mut request,
            ok(vec![chat("Alice"), chat(" Smith")]),
        )
        .await;
    result.unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].starts_with("chat "));
    assert!(sent[0].contains(r#""content":"MASKED""#));
    let seen = harness.assembly.seen_messages.lock().unwrap().clone();
    assert_eq!(
        seen,
        [Some(r#"[{"role":"user","content":"hi"}]"#.to_owned())]
    );
}

#[tokio::test]
async fn passes_no_messages_when_the_request_has_none() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let (_sent, result) = harness
        .run(Kind::Mask, &mut request, ok(vec![chat("Alice")]))
        .await;
    result.unwrap();
    assert_eq!(*harness.assembly.seen_messages.lock().unwrap(), [None]);
}
