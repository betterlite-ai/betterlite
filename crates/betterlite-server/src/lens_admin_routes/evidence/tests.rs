#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::execution::{ExecutionContent, ExecutionSource};

use crate::lens_access::tests::support::{
    Harness, admin, call, execution_id, lens, lens_in_team, request,
};

const TRACING_DISABLED: &str =
    "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.";

#[tokio::test]
async fn the_execution_is_built_from_the_id() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let id = execution_id(&["traces", "team-1", "trace-1"]);
    let reply = call(
        harness.app(),
        request(
            "GET",
            &format!("/lens/l1/executions/{id}"),
            Some(&admin()),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let content: ExecutionContent = reply.json();
    assert_eq!(content.execution.id, id);
    assert_eq!(content.execution.source, ExecutionSource::Traces);
    assert_eq!(content.execution.trace_id, "trace-1");
    assert_eq!(content.execution.name, "trace-1");
    assert_eq!(content.execution.team_id, "team-1");
    assert_eq!(content.execution.trace_ref, "");
    assert_eq!(content.execution.start_time, "");
    assert_eq!(content.execution.span_count, 1);
    assert!(!content.execution.root_seen);
    assert_eq!(content.execution.service, "");
    assert!(content.execution.metadata.is_empty());
    assert!(content.parts.is_empty());
    let storage = harness.storage.as_ref().expect("the storage exists");
    let contents = storage.contents();
    let [parameters] = contents.as_slice() else {
        panic!("one content query");
    };
    assert_eq!(parameters.source, ExecutionSource::Traces);
    assert_eq!(parameters.id, "trace-1");
    assert_eq!(parameters.trace_ref, "");
    assert_eq!(parameters.record_team, "team-1");
    assert_eq!(parameters.cursor, "");
    assert_eq!(parameters.offset, 1);
}

#[tokio::test]
async fn a_request_execution_marks_the_root_seen() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let id = execution_id(&["requests", "team-1", "trace-1", "ref-1"]);
    let reply = call(
        harness.app(),
        request(
            "GET",
            &format!("/lens/l1/executions/{id}"),
            Some(&admin()),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let content: ExecutionContent = reply.json();
    assert_eq!(content.execution.source, ExecutionSource::Requests);
    assert!(content.execution.root_seen);
    assert_eq!(content.execution.trace_ref, "ref-1");
}

#[tokio::test]
async fn the_cursor_and_offset_reach_the_storage() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let id = execution_id(&["traces", "team-1", "trace-1"]);
    let uri = format!("/lens/l1/executions/{id}?cursor=abc&offset=3");
    let reply = call(harness.app(), request("GET", &uri, Some(&admin()), None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let storage = harness.storage.as_ref().expect("the storage exists");
    let contents = storage.contents();
    let [parameters] = contents.as_slice() else {
        panic!("one content query");
    };
    assert_eq!(parameters.cursor, "abc");
    assert_eq!(parameters.offset, 4);
}

#[tokio::test]
async fn a_broken_id_is_404() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request(
            "GET",
            "/lens/l1/executions/not-base64",
            Some(&admin()),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Execution not found");
}

#[tokio::test]
async fn an_unknown_source_is_404() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let id = execution_id(&["logs", "team-1", "trace-1"]);
    let reply = call(
        harness.app(),
        request(
            "GET",
            &format!("/lens/l1/executions/{id}"),
            Some(&admin()),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Execution not found");
}

#[tokio::test]
async fn another_teams_execution_is_404() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_in_team("l1", "team-1"));
    let id = execution_id(&["traces", "team-2", "trace-1"]);
    let reply = call(
        harness.app(),
        request(
            "GET",
            &format!("/lens/l1/executions/{id}"),
            Some(&admin()),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Execution not found");
}

#[tokio::test]
async fn a_missing_lens_is_404_before_the_id_is_parsed() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        request(
            "GET",
            "/lens/nope/executions/not-base64",
            Some(&admin()),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn without_storage_the_route_is_501() {
    let harness = Harness::new().without_storage();
    harness.database.seed_lens(lens("l1"));
    let id = execution_id(&["traces", "team-1", "trace-1"]);
    let reply = call(
        harness.app(),
        request(
            "GET",
            &format!("/lens/l1/executions/{id}"),
            Some(&admin()),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(reply.detail(), TRACING_DISABLED);
}
