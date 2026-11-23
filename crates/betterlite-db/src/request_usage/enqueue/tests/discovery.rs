use std::time::Duration;

use crate::request_usage::discovery::enqueue_tool_registry_upsert;
use crate::tool_discovery::{ToolDiscoveryItem, ToolDiscoveryQueue};

use super::support::names;

const EVERY_SOURCE: &str = r#"{
    "standard_logging_object": {
        "metadata": {
            "mcp_tool_call_metadata": {
                "namespaced_tool_name": "mcp.namespaced",
                "name": "mcp.plain",
                "mcp_server_name": "server-one"
            }
        }
    },
    "tools": [{"function": {"name": "openai_tool"}}],
    "passthrough_logging_payload": {"request_body": {"tools": [{"name": "anthropic_tool"}]}},
    "litellm_params": {"metadata": {"user_api_key_alias": "alias", "user_agent": "agent/1.0"}}
}"#;

fn discovered(items: &[ToolDiscoveryItem]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|item| {
            (
                item.tool_name.clone().unwrap_or_default(),
                item.origin.clone().unwrap_or_default(),
            )
        })
        .collect()
}

#[tokio::test]
async fn discovers_every_tool_source_in_precedence_order() {
    let queue = ToolDiscoveryQueue::new(16).unwrap();
    enqueue_tool_registry_upsert(
        &queue,
        Some(EVERY_SOURCE.as_bytes()),
        Some("kh"),
        Some("t1"),
        &names(&["response_tool"]),
    )
    .await;
    let items = queue.flush();
    assert_eq!(
        discovered(&items),
        vec![
            ("mcp.namespaced".to_owned(), "server-one".to_owned()),
            ("openai_tool".to_owned(), "user_defined".to_owned()),
            ("anthropic_tool".to_owned(), "user_defined".to_owned()),
            ("response_tool".to_owned(), "user_defined".to_owned()),
        ]
    );
    for item in &items {
        assert_eq!(item.key_hash.as_deref(), Some("kh"));
        assert_eq!(item.team_id.as_deref(), Some("t1"));
        assert_eq!(item.key_alias.as_deref(), Some("alias"));
        assert_eq!(item.user_agent.as_deref(), Some("agent/1.0"));
        assert_eq!(item.created_by, None);
    }
}

#[tokio::test]
async fn falls_back_to_the_plain_mcp_name_and_a_user_defined_origin() {
    let kwargs = r#"{"standard_logging_object":{"metadata":{"mcp_tool_call_metadata":{"namespaced_tool_name":"","name":"mcp.plain","mcp_server_name":0}}}}"#;
    let queue = ToolDiscoveryQueue::new(4).unwrap();
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    let items = queue.flush();
    assert_eq!(
        discovered(&items),
        vec![("mcp.plain".to_owned(), "user_defined".to_owned())]
    );
    assert_eq!(items[0].key_hash, None);
    assert_eq!(items[0].key_alias, None);
}

#[tokio::test]
async fn keeps_the_first_occurrence_of_a_name_until_the_flush_resets_it() {
    let kwargs = r#"{
        "standard_logging_object": {"metadata": {"mcp_tool_call_metadata": {"name": "shared", "mcp_server_name": "server-one"}}},
        "tools": [{"function": {"name": "shared"}}]
    }"#;
    let queue = ToolDiscoveryQueue::new(4).unwrap();
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    let first = queue.flush();
    assert_eq!(
        discovered(&first),
        vec![("shared".to_owned(), "server-one".to_owned())]
    );
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    let second = queue.flush();
    assert_eq!(
        discovered(&second),
        vec![("shared".to_owned(), "server-one".to_owned())]
    );
}

#[tokio::test]
async fn a_malformed_tool_name_stops_the_remaining_sources() {
    let kwargs = r#"{
        "tools": [{"function": {"name": "first_tool"}}, {"function": {"name": 5}}],
        "passthrough_logging_payload": {"request_body": {"tools": [{"name": "later_tool"}]}}
    }"#;
    let queue = ToolDiscoveryQueue::new(8).unwrap();
    enqueue_tool_registry_upsert(
        &queue,
        Some(kwargs.as_bytes()),
        None,
        None,
        &names(&["response_tool"]),
    )
    .await;
    assert_eq!(
        discovered(&queue.flush()),
        vec![("first_tool".to_owned(), "user_defined".to_owned())]
    );
}

#[tokio::test]
async fn unreadable_tool_definitions_are_skipped() {
    let kwargs = r#"{
        "tools": [5, {"function": null}, {"function": "x"}, {"function": {"name": ""}}, {"function": {"name": "kept"}}],
        "passthrough_logging_payload": {"request_body": {"tools": [5, {"name": ""}, {"name": "anthropic_kept"}]}}
    }"#;
    let queue = ToolDiscoveryQueue::new(8).unwrap();
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    assert_eq!(
        discovered(&queue.flush()),
        vec![
            ("kept".to_owned(), "user_defined".to_owned()),
            ("anthropic_kept".to_owned(), "user_defined".to_owned()),
        ]
    );
}

#[tokio::test]
async fn malformed_litellm_params_stop_every_source() {
    let kwargs = r#"{"litellm_params":"bad","tools":[{"function":{"name":"openai_tool"}}]}"#;
    let queue = ToolDiscoveryQueue::new(8).unwrap();
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    assert!(queue.flush().is_empty());
}

#[tokio::test]
async fn a_malformed_mcp_source_stops_the_request_tools() {
    let kwargs =
        r#"{"standard_logging_object":"bad","tools":[{"function":{"name":"openai_tool"}}]}"#;
    let queue = ToolDiscoveryQueue::new(8).unwrap();
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    assert!(queue.flush().is_empty());
}

#[tokio::test]
async fn a_falsey_or_unreadable_mcp_metadata_leaves_the_request_tools() {
    let cases = [
        r#"{"standard_logging_object":{"metadata":[]},"tools":[{"function":{"name":"kept"}}]}"#,
        r#"{"standard_logging_object":{"metadata":{"mcp_tool_call_metadata":"bad"}},"tools":[{"function":{"name":"kept"}}]}"#,
        r#"{"standard_logging_object":{"metadata":{"mcp_tool_call_metadata":{}}},"tools":[{"function":{"name":"kept"}}]}"#,
    ];
    for kwargs in cases {
        let queue = ToolDiscoveryQueue::new(8).unwrap();
        enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
        assert_eq!(
            discovered(&queue.flush()),
            vec![("kept".to_owned(), "user_defined".to_owned())]
        );
    }
}

#[tokio::test]
async fn falsey_attribution_values_enqueue_without_them() {
    let kwargs = r#"{"tools":[{"function":{"name":"kept"}}],"litellm_params":{"metadata":{"user_api_key_alias":0,"user_agent":""}}}"#;
    let queue = ToolDiscoveryQueue::new(8).unwrap();
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    let items = queue.flush();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key_alias, None);
    assert_eq!(items[0].user_agent, None);
}

#[tokio::test]
async fn a_malformed_attribution_value_stops_every_source() {
    let kwargs = r#"{"tools":[{"function":{"name":"dropped"}}],"litellm_params":{"metadata":{"user_api_key_alias":5}}}"#;
    let queue = ToolDiscoveryQueue::new(8).unwrap();
    enqueue_tool_registry_upsert(&queue, Some(kwargs.as_bytes()), None, None, &[]).await;
    assert!(queue.flush().is_empty());
}

#[tokio::test]
async fn absent_or_unreadable_kwargs_enqueue_nothing() {
    let queue = ToolDiscoveryQueue::new(8).unwrap();
    enqueue_tool_registry_upsert(&queue, None, None, None, &names(&["response_tool"])).await;
    enqueue_tool_registry_upsert(&queue, Some(b"not json"), None, None, &[]).await;
    assert!(queue.flush().is_empty());
}

#[tokio::test]
async fn a_full_discovery_queue_holds_the_next_tool_for_the_next_flush() {
    let queue = ToolDiscoveryQueue::new(1).unwrap();
    let first = r#"{"tools":[{"function":{"name":"a"}}]}"#;
    enqueue_tool_registry_upsert(&queue, Some(first.as_bytes()), None, None, &[]).await;
    let second = r#"{"tools":[{"function":{"name":"b"}}]}"#;
    let waiting = enqueue_tool_registry_upsert(&queue, Some(second.as_bytes()), None, None, &[]);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), waiting.as_mut())
            .await
            .is_err()
    );
    assert_eq!(
        discovered(&queue.flush()),
        vec![("a".to_owned(), "user_defined".to_owned())]
    );
    waiting.await;
    assert_eq!(
        discovered(&queue.flush()),
        vec![("b".to_owned(), "user_defined".to_owned())]
    );
}
