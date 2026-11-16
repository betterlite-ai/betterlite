#![cfg(test)]

use super::shared::{Harness, Kind, chat, ok, request_data, tokens_request};
use crate::presidio_streams::error::PresidioStreamError;

#[tokio::test]
async fn buffers_the_chunks_and_unmasks_them_with_the_request_tokens() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let events = ok(vec![chat("Hi <PERS"), chat("ON_1>!")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains(r#""content":"Hi Alice!""#));
    let seen = harness.assembly.seen_messages.lock().unwrap().clone();
    assert_eq!(
        seen,
        [Some(r#"[{"role":"user","content":"hi"}]"#.to_owned())]
    );
}

#[tokio::test]
async fn a_request_without_tokens_is_assembled_but_not_changed() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let (sent, result) = harness
        .run(Kind::Unmask, &mut request, ok(vec![chat("Hi <PERSON_1>")]))
        .await;
    result.unwrap();
    assert!(sent[0].contains(r#""content":"Hi <PERSON_1>""#));
    assert_eq!(*harness.assembly.seen_messages.lock().unwrap(), [None]);
}

#[tokio::test]
async fn pii_tokens_of_the_wrong_type_fail_before_anything_is_sent() {
    let harness = Harness::new();
    let mut request = request_data(r#"{"metadata":{"pii_tokens":"nope"}}"#);
    let (sent, result) = harness
        .run(Kind::Unmask, &mut request, ok(vec![chat("a")]))
        .await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::Actions(_)
    ));
    assert!(sent.is_empty());
}
