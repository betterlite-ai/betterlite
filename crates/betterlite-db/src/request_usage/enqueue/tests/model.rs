use rust_decimal::Decimal;

use crate::model_usage::ModelUsageTransaction;
use crate::request_usage::enqueue::enqueue_model_usage;
use crate::spend_queues::entity::SpendAmount;

use super::support::bounded;

const MODEL_PAYLOAD: &str = r#"{"startTime":"2026-01-02T03:04:05Z","metadata":"{}","model":"gpt-4o","model_group":"gpt-4o","custom_llm_provider":"openai","request_tags":"[]","spend":0.25,"prompt_tokens":3,"completion_tokens":4,"status":"success"}"#;

#[tokio::test]
async fn enqueues_a_model_usage_row_with_its_rollup_key() {
    let queue = bounded::<ModelUsageTransaction>(8);
    enqueue_model_usage(&queue, MODEL_PAYLOAD.as_bytes()).await;
    let rows = queue.drain(usize::MAX);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key.date, "2026-01-02");
    assert_eq!(rows[0].key.model, "gpt-4o");
    assert_eq!(rows[0].key.model_group, "gpt-4o");
    assert_eq!(rows[0].key.custom_llm_provider, "openai");
    assert_eq!(rows[0].key.task_type, "uncategorized");
    assert_eq!(rows[0].spend, SpendAmount(Decimal::new(25, 2)));
    assert_eq!(rows[0].prompt_tokens, 3);
    assert_eq!(rows[0].completion_tokens, 4);
    assert!(rows[0].successful);
}

#[tokio::test]
async fn reads_a_failed_request_with_the_source_fallbacks() {
    let payload = r#"{"startTime":"2026-01-02T03:04:05Z","metadata":"{}","request_tags":"[]","spend":0.25,"prompt_tokens":3,"completion_tokens":4,"status":"error"}"#;
    let queue = bounded::<ModelUsageTransaction>(8);
    enqueue_model_usage(&queue, payload.as_bytes()).await;
    let rows = queue.drain(usize::MAX);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key.model, "unknown");
    assert_eq!(rows[0].key.model_group, "unknown");
    assert_eq!(rows[0].key.custom_llm_provider, "unknown");
    assert!(!rows[0].successful);
}

#[tokio::test]
async fn skips_model_usage_for_an_internal_call() {
    let payload = MODEL_PAYLOAD.replace(
        r#""metadata":"{}""#,
        r#""metadata":"{\"internal_call_origin\":\"litellm\"}""#,
    );
    let queue = bounded::<ModelUsageTransaction>(8);
    enqueue_model_usage(&queue, payload.as_bytes()).await;
    assert!(queue.is_empty());
}

#[tokio::test]
async fn drops_an_unreadable_model_usage_payload_without_raising() {
    let payload = MODEL_PAYLOAD.replace(r#","status":"success""#, "");
    let queue = bounded::<ModelUsageTransaction>(8);
    enqueue_model_usage(&queue, payload.as_bytes()).await;
    assert!(queue.is_empty());
}

#[tokio::test]
async fn drops_a_model_usage_row_whose_queue_no_longer_admits_records() {
    let queue = bounded::<ModelUsageTransaction>(8);
    assert_eq!(queue.close(), 0);
    enqueue_model_usage(&queue, MODEL_PAYLOAD.as_bytes()).await;
    assert!(queue.is_empty());
}
