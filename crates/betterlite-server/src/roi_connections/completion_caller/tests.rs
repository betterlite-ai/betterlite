#![cfg(test)]

use betterlite_roi::estimator::models::{ROIResponseFormatType, ReasoningEffort};

use crate::roi_connections::completion_caller::completion_caller;
use crate::roi_connections::error::RoiConnectionsError;
use crate::roi_connections::gateway_key::MISSING_KEY;
use crate::roi_connections::tests::support::{
    Answer, SentCompletion, completion_request, harness, secret, settings,
};
use crate::roi_github::error::SourceError;

#[tokio::test]
async fn completion_caller_posts_the_request_without_none_fields() {
    let harness = harness().answers(vec![Answer::ok("{\"id\":\"completion-1\"}")]);
    let caller = completion_caller(&settings(), &harness.state()).expect("caller");
    let object = caller(completion_request(None)).await.expect("completion");
    assert_eq!(object.get(), "{\"id\":\"completion-1\"}");
    let seen = harness.wire.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, "http://litellm.internal/v1/chat/completions");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer sk-estimator")
    );
    let sent: SentCompletion = serde_json::from_slice(&seen[0].body).expect("sent completion");
    assert_eq!(sent.model, "gpt-4o");
    assert_eq!(sent.temperature, 0);
    assert_eq!(sent.max_tokens, 1200);
    assert_eq!(sent.messages[0].content, "evidence");
    assert_eq!(
        sent.response_format.r#type,
        ROIResponseFormatType::JsonObject
    );
    assert_eq!(sent.metadata.tags, ["litellm-roi-estimator"]);
    assert!(sent.metadata.litellm_roi_estimator);
    assert_eq!(sent.reasoning_effort, None);
    let text = String::from_utf8(seen[0].body.clone()).expect("body text");
    assert!(!text.contains("reasoning_effort"));
}

#[tokio::test]
async fn completion_caller_sends_the_reasoning_effort() {
    let harness = harness().answers(vec![Answer::ok("{}")]);
    let caller = completion_caller(&settings(), &harness.state()).expect("caller");
    caller(completion_request(Some(ReasoningEffort::None)))
        .await
        .expect("completion");
    let seen = harness.wire.seen();
    let text = String::from_utf8(seen[0].body.clone()).expect("body text");
    assert!(text.contains("\"reasoning_effort\":\"none\""));
}

#[tokio::test]
async fn completion_caller_maps_an_error_status() {
    let harness = harness().answers(vec![Answer::status(500)]);
    let caller = completion_caller(&settings(), &harness.state()).expect("caller");
    let error = caller(completion_request(None))
        .await
        .expect_err("completion failure");
    assert!(matches!(error, SourceError::Http { status: 500, .. }));
}

#[tokio::test]
async fn completion_caller_rejects_a_non_json_answer() {
    let harness = harness().answers(vec![Answer::ok("not json")]);
    let caller = completion_caller(&settings(), &harness.state()).expect("caller");
    let error = caller(completion_request(None))
        .await
        .expect_err("completion failure");
    assert!(matches!(error, SourceError::Pagination { .. }));
}

#[tokio::test]
async fn completion_caller_maps_an_invalid_credential() {
    let mut roi = settings();
    roi.estimator_key = secret("bad\nkey");
    let harness = harness().answers(vec![Answer::ok("{}")]);
    let caller = completion_caller(&roi, &harness.state()).expect("caller");
    let error = caller(completion_request(None))
        .await
        .expect_err("completion failure");
    assert!(matches!(error, SourceError::Token(_)));
}

#[tokio::test]
async fn completion_caller_uses_the_master_key_when_the_estimator_key_is_empty() {
    let mut roi = settings();
    roi.estimator_key = secret("");
    let harness = harness()
        .master_key("master-key")
        .answers(vec![Answer::ok("{}")]);
    let caller = completion_caller(&roi, &harness.state()).expect("caller");
    caller(completion_request(None)).await.expect("completion");
    let seen = harness.wire.seen();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer master-key"));
}

#[test]
fn completion_caller_requires_a_credential() {
    let mut roi = settings();
    roi.estimator_key = secret("");
    let harness = harness();
    let error = completion_caller(&roi, &harness.state())
        .err()
        .expect("missing key");
    assert!(matches!(error, RoiConnectionsError::Conflict(MISSING_KEY)));
}
