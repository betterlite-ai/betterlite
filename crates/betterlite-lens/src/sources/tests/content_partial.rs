#![cfg(test)]

use crate::models::execution::{Execution, ExecutionSource};
use crate::models::settings::Scope;
use crate::sources::part_row::PartRow;
use crate::sources::source_reader::SourceReader;
use crate::sources::stub_storage::StubStorage;

fn execution(root_seen: bool) -> Execution {
    Execution {
        id: "exec-1".to_owned(),
        source: ExecutionSource::Traces,
        trace_id: "t1".to_owned(),
        trace_ref: String::new(),
        team_id: "team-a".to_owned(),
        name: "run".to_owned(),
        start_time: "2026-01-01T00:00:00Z".to_owned(),
        span_count: 2,
        root_seen,
        service: String::new(),
        metadata: vec![],
    }
}

fn rows_with_truncated(truncated: i64) -> Vec<u8> {
    let row = PartRow {
        span_id: "s1".to_owned(),
        parent_span_id: String::new(),
        name: "root".to_owned(),
        kind: "agent".to_owned(),
        content: "hi".to_owned(),
        truncated,
    };
    serde_json::to_vec(&[row]).expect("encodes")
}

async fn partial_for(root_seen: bool, truncated: i64) -> bool {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows_with_truncated(truncated)),
    };
    reader
        .content(&Scope::default(), &execution(root_seen), "", 0)
        .await
        .expect("ok")
        .partial
}

#[tokio::test]
async fn is_false_only_when_root_was_seen_and_nothing_was_truncated() {
    assert!(!partial_for(true, 0).await);
}

#[tokio::test]
async fn is_true_when_root_was_not_seen_even_without_truncation() {
    assert!(partial_for(false, 0).await);
}

#[tokio::test]
async fn is_true_when_truncated_even_though_root_was_seen() {
    assert!(partial_for(true, 1).await);
}

#[tokio::test]
async fn is_true_when_both_conditions_hold() {
    assert!(partial_for(false, 1).await);
}
