use std::sync::Arc;

use betterlite_logging::rubrik_delivery::queue::queue_snapshot;
use betterlite_logging::rubrik_event_logging::failure_hook::async_post_call_failure_hook;
use betterlite_logging::rubrik_moderation::context::{LoggingObjectStash, call_details};
use betterlite_models::decisions::Json;
use tracing::Level;

use super::support::{
    Harness, details, exception, field, json, logging_object, object_keys, request_data, text,
};

const BLOCKED: &str = r#"{"_rubrik_blocked":true,"litellm_call_id":"call-1","model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"standard_logging_object":{"id":"chatcmpl-provider","status":"success"}}"#;

#[tokio::test]
async fn a_request_that_did_not_raise_a_block_is_ignored() {
    let harness = Harness::new(&[], &[]);
    let object = logging_object(BLOCKED);
    let mut data = request_data(LoggingObjectStash::Present(Some(Arc::clone(&object))));
    let metadata = details("{}");
    async_post_call_failure_hook(&harness.logger, &mut data, None, &metadata).await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    assert!(matches!(
        data.logging_obj,
        LoggingObjectStash::Present(Some(_))
    ));
}

#[tokio::test]
async fn a_block_from_another_guardrail_is_ignored() {
    let harness = Harness::new(&[], &[]);
    let object = logging_object(BLOCKED);
    let mut data = request_data(LoggingObjectStash::Present(Some(Arc::clone(&object))));
    let raised = exception("blocked by policy", "another");
    let metadata = details("{}");
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    assert!(harness.logs.events().is_empty());
    assert!(matches!(
        data.logging_obj,
        LoggingObjectStash::Present(Some(_))
    ));
}

#[tokio::test]
async fn the_raising_guardrail_logs_the_block() {
    let harness = Harness::new(&[], &[]);
    let object = logging_object(BLOCKED);
    let mut data = request_data(LoggingObjectStash::Present(Some(Arc::clone(&object))));
    let raised = exception("blocked by policy", "rubrik");
    let metadata = details("{}");
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    let queue = queue_snapshot(&harness.logger);
    assert_eq!(queue.len(), 1);
    assert_eq!(
        text(field(&queue[0], "response")),
        "ModifyResponseException: blocked by policy"
    );
    assert!(matches!(data.logging_obj, LoggingObjectStash::Missing));
}

#[tokio::test]
async fn a_zero_sampling_rate_still_logs_a_block() {
    let harness = Harness::new(&[("RUBRIK_SAMPLING_RATE", "0")], &[]);
    let object = logging_object(BLOCKED);
    let mut data = request_data(LoggingObjectStash::Present(Some(Arc::clone(&object))));
    let raised = exception("blocked by policy", "rubrik");
    let metadata = details("{}");
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    assert_eq!(queue_snapshot(&harness.logger).len(), 1);
}

#[tokio::test]
async fn a_missing_stash_warns_and_logs_nothing() {
    let harness = Harness::new(&[], &[]);
    let mut data = request_data(LoggingObjectStash::Missing);
    let raised = exception("blocked by policy", "rubrik");
    let metadata = details(r#"{"user_api_key_user_id":"u-1"}"#);
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    let warnings = harness.logs.with_level(Level::WARN);
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].message,
        "Rubrik: block exception without stashed logging_obj"
    );
    assert_eq!(warnings[0].litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(warnings[0].model.as_deref(), Some("gpt-4o"));
    assert_eq!(warnings[0].user_id.as_deref(), Some("u-1"));
}

#[tokio::test]
async fn a_stash_without_a_logging_object_warns_and_logs_nothing() {
    let harness = Harness::new(&[], &[]);
    let mut data = request_data(LoggingObjectStash::Present(None));
    let raised = exception("blocked by policy", "rubrik");
    let metadata = details("{}");
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    let warnings = harness.logs.with_level(Level::WARN);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].user_id.as_deref(), Some("None"));
}

#[tokio::test]
async fn the_blocked_flag_stays_on_the_call_details() {
    let harness = Harness::new(&[], &[]);
    let object = logging_object(BLOCKED);
    let mut data = request_data(LoggingObjectStash::Present(Some(Arc::clone(&object))));
    let raised = exception("blocked by policy", "rubrik");
    let metadata = details("{}");
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    let details = call_details(Some(&object));
    assert_eq!(details.get("_rubrik_blocked"), Some(&Json::Bool(true)));
}

#[tokio::test]
async fn an_unreadable_payload_is_logged_and_not_queued() {
    let harness = Harness::new(&[], &[]);
    let object = logging_object(
        r#"{"litellm_call_id":"call-1","standard_logging_object":"chatcmpl-provider"}"#,
    );
    let mut data = request_data(LoggingObjectStash::Present(Some(Arc::clone(&object))));
    let raised = exception("blocked by policy", "rubrik");
    let metadata = details("{}");
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "Rubrik: failed to build the blocked-tool payload; the event will not be logged"
    );
    assert_eq!(errors[0].litellm_call_id.as_deref(), Some("call-1"));
}

#[tokio::test]
async fn the_caller_metadata_reaches_the_block_payload() {
    let harness = Harness::new(&[], &[]);
    let object = logging_object(
        r#"{"litellm_call_id":"call-1","model":"gpt-4o","messages":[],"start_time":"2026-01-01T00:00:00Z"}"#,
    );
    let mut data = request_data(LoggingObjectStash::Present(Some(Arc::clone(&object))));
    let raised = exception("blocked by policy", "rubrik");
    let metadata = details(r#"{"user_api_key_user_id":"u-1","user_api_key_alias":"alias"}"#);
    async_post_call_failure_hook(&harness.logger, &mut data, Some(&raised), &metadata).await;
    let queue = queue_snapshot(&harness.logger);
    assert_eq!(queue.len(), 1);
    assert_eq!(
        object_keys(field(&queue[0], "metadata")),
        ["user_api_key_user_id", "user_api_key_alias"]
    );
    assert_eq!(
        field(&queue[0], "metadata"),
        &json(r#"{"user_api_key_user_id":"u-1","user_api_key_alias":"alias"}"#)
    );
}
