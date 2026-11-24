use std::sync::Arc;

use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;

#[tokio::test]
async fn a_rate_limited_frame_answers_once_and_calls_no_provider() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    let failure = support::QuotaFailure::RateLimit("quota exceeded");
    let quota = support::ScriptedQuota::new(&shared, Some(failure));
    connection.quota_callbacks = Some(vec![Arc::new(quota)]);
    let mut handler = support::handler(connection);
    let provider = support::ScriptedProvider::idle(&shared);

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let error_frame = r#"{"type": "error", "error": {"type": "rate_limit_exceeded", "message": "quota exceeded"}}"#;
    assert_eq!(support::frames(&shared), vec![support::json(error_frame)]);
    assert!(support::calls(&shared).is_empty());
    assert_eq!(support::trace(&shared), ["quota"]);

    let recorded = support::quotas(&shared);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].model.as_deref(), Some(support::MODEL));
    assert_eq!(recorded[0].input_tokens, 1);
    assert_eq!(recorded[0].output_tokens, 1024);
    assert!(logs.is_empty());
    assert_eq!(support::history(&handler), support::json("{}"));
}

#[tokio::test]
async fn an_empty_model_group_still_charges_the_connection_model() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    connection.litellm_metadata = Some(support::object(r#"{"deployment_model_name": ""}"#));
    let failure = support::QuotaFailure::RateLimit("quota exceeded");
    let quota = support::ScriptedQuota::new(&shared, Some(failure));
    connection.quota_callbacks = Some(vec![Arc::new(quota)]);
    let mut handler = support::handler(connection);
    let provider = support::ScriptedProvider::idle(&shared);

    let (outcome, _logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let recorded = support::quotas(&shared);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].model.as_deref(), Some(support::MODEL));
}
