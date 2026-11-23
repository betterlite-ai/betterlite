use betterlite_models::tool_spend::ToolUsageTransaction;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::request_usage::enqueue::{ToolUsageError, enqueue_tool_usage, tool_usage_transaction};

use super::support::{bounded, names};

const TOOL_PAYLOAD: &str = r#"{"request_id":"req-tool","startTime":"2026-01-02T03:04:05Z","spend":0.25,"total_tokens":10,"mcp_namespaced_tool_name":"mcp.search"}"#;

const TOOL_PAYLOAD_WITHOUT_MCP: &str = r#"{"request_id":"req-tool","startTime":"2026-01-02T03:04:05Z","spend":0.25,"total_tokens":10}"#;

#[tokio::test]
async fn enqueues_one_tool_usage_row_for_every_tool_name_source() {
    let queue = bounded::<ToolUsageTransaction>(8);
    enqueue_tool_usage(
        &queue,
        TOOL_PAYLOAD.as_bytes(),
        &names(&["response_tool"]),
        &names(&["realtime_tool"]),
    )
    .await;
    let rows = queue.drain(usize::MAX);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].tool_names,
        names(&["mcp.search", "response_tool", "realtime_tool"])
    );
    assert_eq!(rows[0].request_id, "req-tool");
}

#[tokio::test]
async fn skips_tool_usage_when_the_request_named_no_tool() {
    let queue = bounded::<ToolUsageTransaction>(8);
    enqueue_tool_usage(&queue, TOOL_PAYLOAD_WITHOUT_MCP.as_bytes(), &[], &[]).await;
    assert!(queue.is_empty());
}

#[tokio::test]
async fn drops_tool_usage_for_a_start_time_the_source_cannot_decode() {
    let payload = TOOL_PAYLOAD.replace("2026-01-02T03:04:05Z", "not-a-time");
    let queue = bounded::<ToolUsageTransaction>(8);
    enqueue_tool_usage(&queue, payload.as_bytes(), &[], &[]).await;
    assert!(queue.is_empty());
}

#[tokio::test]
async fn drops_an_unreadable_tool_usage_payload_without_raising() {
    let queue = bounded::<ToolUsageTransaction>(8);
    enqueue_tool_usage(&queue, b"not json", &[], &[]).await;
    assert!(queue.is_empty());
}

#[tokio::test]
async fn drops_a_tool_usage_row_whose_queue_no_longer_admits_records() {
    let queue = bounded::<ToolUsageTransaction>(8);
    assert_eq!(queue.close(), 0);
    enqueue_tool_usage(&queue, TOOL_PAYLOAD.as_bytes(), &names(&["resp"]), &[]).await;
    assert!(queue.is_empty());
}

#[rstest]
#[case::request_id(
    r#"{"startTime":"2026-01-02T03:04:05Z","spend":0.25,"total_tokens":10}"#,
    "request_id"
)]
#[case::spend(
    r#"{"request_id":"r","startTime":"2026-01-02T03:04:05Z","total_tokens":10}"#,
    "spend"
)]
#[case::total_tokens(
    r#"{"request_id":"r","startTime":"2026-01-02T03:04:05Z","spend":0.25}"#,
    "total_tokens"
)]
#[case::start_time(r#"{"request_id":"r","spend":0.25,"total_tokens":10}"#, "startTime")]
#[test]
fn tool_usage_transaction_names_each_missing_indexed_field(
    #[case] payload: &str,
    #[case] field: &str,
) {
    let error = tool_usage_transaction(payload.as_bytes(), &[], &[]).unwrap_err();
    assert!(matches!(error, ToolUsageError::Field { field: expected } if expected == field));
}

#[test]
fn tool_usage_transaction_reads_the_row_the_source_indexes() {
    let transaction = tool_usage_transaction(
        TOOL_PAYLOAD.as_bytes(),
        &names(&["response_tool"]),
        &names(&["realtime_tool"]),
    )
    .unwrap()
    .unwrap();
    assert_eq!(transaction.request_id, "req-tool");
    assert_eq!(transaction.date, "2026-01-02");
    assert_eq!(
        transaction.tool_names,
        names(&["mcp.search", "response_tool", "realtime_tool"])
    );
    assert_eq!(transaction.spend, Decimal::new(25, 2));
    assert_eq!(transaction.total_tokens, 10);
}

#[test]
fn tool_usage_transaction_rejects_a_payload_that_is_not_json() {
    let error = tool_usage_transaction(b"not json", &[], &[]).unwrap_err();
    assert!(matches!(error, ToolUsageError::Payload(_)));
}
