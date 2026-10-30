#![cfg(test)]

use crate::models::execution::{Execution, ExecutionSource};
use crate::models::settings::Scope;
use crate::sources::source_reader::SourceReader;
use crate::sources::stub_storage::StubStorage;

fn execution() -> Execution {
    Execution {
        id: "exec-1".to_owned(),
        source: ExecutionSource::Traces,
        trace_id: "t1".to_owned(),
        trace_ref: String::new(),
        team_id: "team-a".to_owned(),
        name: "run".to_owned(),
        start_time: "2026-01-01T00:00:00Z".to_owned(),
        span_count: 2,
        root_seen: true,
        service: String::new(),
        metadata: vec![],
    }
}

#[tokio::test]
async fn sends_the_execution_through_the_storage_parameters() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"[]".to_vec()),
    };
    reader
        .content(&Scope::default(), &execution(), "", 0)
        .await
        .expect("ok");
    let seen = reader
        .storage
        .seen_content
        .lock()
        .expect("lock")
        .clone()
        .expect("storage was called");
    assert_eq!(seen.id, "t1");
    assert_eq!(seen.record_team, "team-a");
    assert_eq!(seen.offset, 1);
}

#[tokio::test]
async fn constructs_trace_parts_from_rows() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(
            br#"[{"span_id": "s1", "parent_span_id": "", "name": "root", "kind": "agent",
                "content": "hi", "truncated": 0}]"#
                .to_vec(),
        ),
    };
    let content = reader
        .content(&Scope::default(), &execution(), "", 0)
        .await
        .expect("ok");
    assert_eq!(content.parts.len(), 1);
    let part = content.parts.first().expect("one part");
    assert_eq!(part.execution_id, "exec-1");
    assert_eq!(part.span_id, "s1");
    assert!(!part.truncated);
}

#[tokio::test]
async fn propagates_a_decode_error() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"not json".to_vec()),
    };
    let result = reader.content(&Scope::default(), &execution(), "", 0).await;
    result.unwrap_err();
}
