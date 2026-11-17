use crate::otel::support::{
    anchored, event, failure, finished, harness, inference, mcp_list, mcp_tool, mcp_tool_failure,
    names, owned_http, success, with_id,
};

#[tokio::test]
async fn dispatch_emits_one_mcp_tool_span_and_skips_the_llm_path() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    assert_eq!(
        names(&spans).first().map(String::as_str),
        Some("tools/call search")
    );
}

#[tokio::test]
async fn dispatch_emits_one_mcp_list_span_for_a_list_tools_call() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(mcp_list(), "call-list"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    assert_eq!(
        names(&spans).first().map(String::as_str),
        Some("tools/list")
    );
}

#[tokio::test]
async fn dispatch_emits_mcp_spans_on_the_failure_event_too() {
    let harness = harness(owned_http(), anchored());
    failure(
        &harness.logger,
        event(with_id(mcp_tool_failure(), "call-mcp")),
    )
    .await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    assert_eq!(
        names(&spans).first().map(String::as_str),
        Some("tools/call search")
    );
}

#[tokio::test]
async fn dispatch_falls_through_to_the_llm_path_for_an_inference_payload() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    assert!(
        names(&spans)
            .first()
            .is_some_and(|name| name.contains("gpt-4o"))
    );
}
