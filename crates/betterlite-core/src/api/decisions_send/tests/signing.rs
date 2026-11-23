use std::future::ready;
use std::thread;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use tokio::runtime::Handle;

use super::support::*;
use crate::api::decisions_send::signing::{BlockingSigning, SigningSeam as _};

// signing: content when the config signed a body, JSON when it did not, and the async AWS split

#[test]
fn signing_signed_body_is_sent_as_content() {
    let mut fixture = Fixture::new();
    fixture.config.signed_body = Some(b"{\"signed\":true}".to_vec());
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    assert_eq!(requests[0].body, b"{\"signed\":true}".to_vec());
}

#[test]
fn signing_unsigned_body_is_sent_as_json() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    let sent = serde_json::to_string(&decisions.body).expect("the wire serializes");
    assert_eq!(requests[0].body, sent.into_bytes());
}

#[test]
fn signing_sync_signs_inline_even_for_aws() {
    let mut fixture = Fixture::new();
    fixture.config.aws = true;
    let decisions = systemone_call("strands_decider", None);
    let caller = thread::current().id();
    fixture.send(&decisions).expect("the call answers");
    assert_eq!(fixture.recording.seen_signs()[0].thread, caller);
}

#[tokio::test]
async fn signing_async_signs_inline_without_aws() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.asend(&decisions).await.expect("the call answers");
    let events = fixture.recording.events.recorded();
    assert!(events.contains(&String::from("sign")));
    assert!(!events.contains(&String::from("off-loop")));
}

#[tokio::test]
async fn signing_async_signs_off_the_request_task_with_aws() {
    let mut fixture = Fixture::new();
    fixture.config.aws = true;
    let decisions = systemone_call("strands_decider", None);
    fixture.asend(&decisions).await.expect("the call answers");
    let events = fixture.recording.events.recorded();
    assert!(events.contains(&String::from("off-loop")));
    assert_eq!(
        fixture.recording.seen_signs().len(),
        1,
        "the config signs once, from the seam's task"
    );
}

#[tokio::test]
async fn signing_blocking_seam_stays_inline_on_a_current_thread_runtime() {
    let signed = BlockingSigning.sign(true, &|| (Headers::new(), None)).await;
    assert_eq!(signed, (Headers::new(), None));
}

// Handle::block_on panics on a runtime worker unless block_in_place marked the thread as blocking, so the closure proves where it ran
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_blocking_seam_signs_inside_the_blocking_region() {
    let sign = || {
        Handle::current().block_on(ready(()));
        (Headers::new(), Some(b"{\"signed\":true}".to_vec()))
    };
    let signed = BlockingSigning.sign(true, &sign).await;
    assert_eq!(
        signed,
        (Headers::new(), Some(b"{\"signed\":true}".to_vec()))
    );
}

#[test]
fn signing_context_carries_the_call_inputs() {
    let mut fixture = Fixture::new();
    let mut decisions = systemone_call("typesafe", None);
    decisions.litellm_params.insert(
        String::from("aws_secret_access_key"),
        Json::Text(String::from("secret")),
    );
    fixture.send(&decisions).expect("the call answers");
    let signed = &fixture.recording.seen_signs()[0];
    assert_eq!(signed.url, "http://127.0.0.1:9/v1/systemone");
    assert_eq!(signed.api_base, "http://127.0.0.1:9");
    assert_eq!(signed.api_key.as_deref(), Some("test-key"));
    assert_eq!(signed.params, decisions.litellm_params);
}
