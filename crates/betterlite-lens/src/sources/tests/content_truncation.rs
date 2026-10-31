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

#[tokio::test]
async fn root_not_seen_marks_the_content_partial() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows_with_truncated(0)),
    };
    let content = reader
        .content(&Scope::default(), &execution(false), "", 0)
        .await
        .expect("ok");
    assert!(content.partial);
}

#[tokio::test]
async fn a_truncated_span_marks_the_content_partial() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows_with_truncated(1)),
    };
    let content = reader
        .content(&Scope::default(), &execution(true), "", 0)
        .await
        .expect("ok");
    assert!(content.partial);
}

#[tokio::test]
async fn a_nonzero_truncated_value_other_than_one_still_marks_partial() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows_with_truncated(7)),
    };
    let content = reader
        .content(&Scope::default(), &execution(true), "", 0)
        .await
        .expect("ok");
    assert!(content.partial);
}
