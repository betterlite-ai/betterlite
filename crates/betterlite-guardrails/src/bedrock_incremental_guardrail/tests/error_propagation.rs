#![cfg(test)]

use super::support::{Harness, Reply, SESSION, inputs};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;

#[tokio::test]
async fn a_closed_transport_becomes_an_http_failure() {
    let mut harness = Harness::new(vec![]);
    harness.transport.close();

    let failure = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect_err("the transport is closed");

    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a transport failure is a plain http failure");
    };
    assert_eq!(status, 500);
    assert!(!detail.is_empty(), "the detail names the failure");
}

#[tokio::test]
async fn a_failed_incremental_scan_keeps_its_status() {
    let mut harness = Harness::new(vec![Reply::failed(
        400,
        r#"{ "message": "invalid content" }"#,
    )])
    .with_session();
    harness.incremental();
    harness.cache.remember(SESSION, &["hello"]);

    let failure = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect_err("the incremental scan fails");

    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a rejected incremental request is a plain http failure");
    };
    assert_eq!(status, 400);
    assert!(detail.contains("invalid content"), "{detail}");
    assert_eq!(harness.transport.count(), 1);
    assert!(harness.cache.ttls().is_empty());
}

#[tokio::test]
async fn a_non_200_reply_keeps_its_status() {
    let mut harness = Harness::new(vec![Reply::failed(
        400,
        r#"{ "message": "invalid content" }"#,
    )]);

    let failure = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect_err("the reply fails the call");

    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a rejected request is a plain http failure");
    };
    assert_eq!(status, 400);
    assert!(detail.contains("invalid content"), "{detail}");
    assert!(!detail.contains("Bedrock guardrail failed"), "{detail}");
}
