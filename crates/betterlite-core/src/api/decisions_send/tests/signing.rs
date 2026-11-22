use std::thread;

use super::support::*;
use crate::api::decisions_send::clients::DecisionsBody;

// signing: content when the config signed a body, JSON when it did not, and the async AWS split

#[test]
fn signing_signed_body_is_sent_as_content() {
    let mut fixture = Fixture::new();
    fixture.config.signed_body = Some(b"{\"signed\":true}".to_vec());
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    match &requests[0].body {
        DecisionsBody::Signed(bytes) => assert_eq!(bytes, b"{\"signed\":true}"),
        DecisionsBody::Json(_) => panic!("a signed body must be sent as content"),
    }
}

#[test]
fn signing_unsigned_body_is_sent_as_json() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    match &requests[0].body {
        DecisionsBody::Json(bytes) => {
            let sent = serde_json::to_string(&decisions.body).expect("the wire serializes");
            assert_eq!(bytes, sent.as_bytes());
        }
        DecisionsBody::Signed(_) => panic!("an unsigned result must be sent as JSON"),
    }
}

#[test]
fn signing_sync_signs_inline_even_for_aws() {
    let mut fixture = Fixture::new();
    fixture.config.aws = true;
    let decisions = systemone_call("strands_decider", None);
    let caller = thread::current().id();
    fixture.send(&decisions).expect("the call answers");
    assert_eq!(fixture.recording.sign_threads(), vec![caller]);
}

#[tokio::test]
async fn signing_async_signs_inline_without_aws() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let caller = thread::current().id();
    fixture.asend(&decisions).await.expect("the call answers");
    assert_eq!(fixture.recording.sign_threads(), vec![caller]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_async_signs_off_the_request_task_with_aws() {
    let mut fixture = Fixture::new();
    fixture.config.aws = true;
    let decisions = systemone_call("strands_decider", None);
    let caller = thread::current().id();
    fixture.asend(&decisions).await.expect("the call answers");
    let threads = fixture.recording.sign_threads();
    assert_eq!(threads.len(), 1);
    assert_ne!(threads[0], caller);
}

#[test]
fn signing_context_carries_the_call_inputs() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let signed = &fixture.recording.seen_signs()[0];
    assert_eq!(signed.url, "http://127.0.0.1:9/v1/systemone");
    assert_eq!(signed.api_base, "http://127.0.0.1:9");
    assert_eq!(signed.api_key.as_deref(), Some("test-key"));
    assert!(signed.params_empty);
}
