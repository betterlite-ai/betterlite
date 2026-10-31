#![cfg(test)]

use crate::models::execution::{Execution, ExecutionSource};
use crate::models::finding::{Evidence, Role};
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

fn evidence() -> Evidence {
    Evidence {
        execution_id: "exec-1".to_owned(),
        span_id: "s1".to_owned(),
        quote: "answer".to_owned(),
        role: Role::Support,
    }
}

#[tokio::test]
async fn returns_true_when_a_row_has_a_positive_count() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(br#"[{"count": 3}]"#.to_vec()),
    };
    let found = reader
        .verify_evidence(&Scope::default(), &execution(), &evidence())
        .await
        .expect("ok");
    assert!(found);
}

#[tokio::test]
async fn returns_false_when_storage_is_empty() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"[]".to_vec()),
    };
    let found = reader
        .verify_evidence(&Scope::default(), &execution(), &evidence())
        .await
        .expect("ok");
    assert!(!found);
}

#[tokio::test]
async fn returns_false_when_the_count_is_zero() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(br#"[{"count": 0}]"#.to_vec()),
    };
    let found = reader
        .verify_evidence(&Scope::default(), &execution(), &evidence())
        .await
        .expect("ok");
    assert!(!found);
}

#[tokio::test]
async fn sends_the_quote_and_span_through_the_storage_parameters() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"[]".to_vec()),
    };
    reader
        .verify_evidence(&Scope::default(), &execution(), &evidence())
        .await
        .expect("ok");
    let seen = reader
        .storage
        .seen_evidence
        .lock()
        .expect("lock")
        .clone()
        .expect("storage was called");
    assert_eq!(seen.span, "s1");
    assert_eq!(seen.quote, "answer");
}
