#![cfg(test)]

use rust_decimal::Decimal;

use super::SampleWindow;
use crate::models::execution::ExecutionSource;
use crate::models::settings::{LensSettings, Scope, Source};
use crate::sources::source_reader::SourceReader;
use crate::sources::stub_storage::StubStorage;

fn settings() -> LensSettings {
    LensSettings {
        name: "lens".to_owned(),
        context: String::new(),
        source: Source::Traces,
        lookback_hours: 24,
        service: String::new(),
        agent_name: String::new(),
        filters: vec![],
        checks: vec![],
        model: "model".to_owned(),
        enabled: true,
        interval_minutes: 15,
        sample_size: None,
        sample_percent: Decimal::ONE_HUNDRED,
        concurrency: 8,
        team_id: "team-a".to_owned(),
        execution_ids: vec![],
        monthly_budget: Decimal::ONE_HUNDRED,
    }
}

fn window() -> SampleWindow {
    SampleWindow {
        start: 0,
        end: 1000,
        offset: 0,
        page_size: 100,
        preview: false,
        cursor: String::new(),
    }
}

#[tokio::test]
async fn sends_scope_and_settings_through_the_storage_parameters() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"[]".to_vec()),
    };
    let scope = Scope {
        team_id: "team-a".to_owned(),
        api_key_hash: "hash-1".to_owned(),
        all_teams: false,
    };
    reader
        .sample(&scope, &settings(), window())
        .await
        .expect("ok");
    let seen = reader
        .storage
        .seen_sample
        .lock()
        .expect("lock")
        .clone()
        .expect("storage was called");
    assert_eq!(seen.base.team, "team-a");
    assert_eq!(seen.base.key_hash, "hash-1");
    assert_eq!(seen.limit, 100);
    assert_eq!(seen.selected_team, "team-a");
}

#[tokio::test]
async fn constructs_execution_records_from_rows() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(
            br#"[{"source": "traces", "trace_id": "t1", "team_id": "team-a", "name": "run",
                "start_time": "2026-01-01T00:00:00Z", "span_count": 2, "root_seen": 1,
                "eligible": 1, "selected": 1}]"#
                .to_vec(),
        ),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window())
        .await
        .expect("ok");
    assert_eq!(sample.executions.len(), 1);
    let execution = sample.executions.first().expect("one execution");
    assert_eq!(execution.source, ExecutionSource::Traces);
    assert_eq!(execution.trace_id, "t1");
    assert!(execution.root_seen);
    assert!(!execution.id.is_empty());
}

#[tokio::test]
async fn rejects_an_invalid_execution_id_filter() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"[]".to_vec()),
    };
    let mut filtered = settings();
    filtered.execution_ids = vec!["not-valid-base64!!!".to_owned()];
    let result = reader.sample(&Scope::default(), &filtered, window()).await;
    result.unwrap_err();
}

#[tokio::test]
async fn propagates_a_decode_error() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"not json".to_vec()),
    };
    let result = reader
        .sample(&Scope::default(), &settings(), window())
        .await;
    result.unwrap_err();
}
