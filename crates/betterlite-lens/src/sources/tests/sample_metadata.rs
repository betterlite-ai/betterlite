#![cfg(test)]

use rust_decimal::Decimal;

use crate::models::execution::ExecutionSource;
use crate::models::settings::{LensSettings, Scope, Source};
use crate::sources::execution_row::ExecutionRow;
use crate::sources::source_reader::SourceReader;
use crate::sources::source_reader_sample::SampleWindow;
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
        team_id: String::new(),
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

fn row_with_attributes(attributes: Vec<(String, String)>) -> Vec<u8> {
    let row = ExecutionRow {
        selection_key: String::new(),
        source: ExecutionSource::Traces,
        trace_id: "t1".to_owned(),
        trace_ref: String::new(),
        team_id: "team-a".to_owned(),
        name: "run".to_owned(),
        start_time: "2026-01-01T00:00:00Z".to_owned(),
        span_count: 1,
        root_seen: 1,
        eligible: 1,
        selected: 1,
        service: String::new(),
        attributes,
    };
    serde_json::to_vec(&[row]).expect("encodes")
}

#[tokio::test]
async fn hides_the_litellm_api_key_hash_attribute() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(row_with_attributes(vec![
            ("litellm.api_key_hash".to_owned(), "secret".to_owned()),
            ("user.id".to_owned(), "alice".to_owned()),
        ])),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window())
        .await
        .expect("ok");
    let metadata = &sample.executions.first().expect("one execution").metadata;
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata.first().expect("one entry").key, "user.id");
}

#[tokio::test]
async fn drops_an_empty_key() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(row_with_attributes(vec![(
            String::new(),
            "value".to_owned(),
        )])),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window())
        .await
        .expect("ok");
    assert!(
        sample
            .executions
            .first()
            .expect("one execution")
            .metadata
            .is_empty()
    );
}

#[tokio::test]
async fn drops_an_oversized_key() {
    let oversized_key = "k".repeat(201);
    let reader = SourceReader {
        storage: StubStorage::with_bytes(row_with_attributes(vec![(
            oversized_key,
            "value".to_owned(),
        )])),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window())
        .await
        .expect("ok");
    assert!(
        sample
            .executions
            .first()
            .expect("one execution")
            .metadata
            .is_empty()
    );
}

#[tokio::test]
async fn drops_an_oversized_value() {
    let oversized_value = "v".repeat(501);
    let reader = SourceReader {
        storage: StubStorage::with_bytes(row_with_attributes(vec![(
            "k1".to_owned(),
            oversized_value,
        )])),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window())
        .await
        .expect("ok");
    assert!(
        sample
            .executions
            .first()
            .expect("one execution")
            .metadata
            .is_empty()
    );
}

#[tokio::test]
async fn drops_an_empty_value() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(row_with_attributes(vec![(
            "k1".to_owned(),
            String::new(),
        )])),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window())
        .await
        .expect("ok");
    assert!(
        sample
            .executions
            .first()
            .expect("one execution")
            .metadata
            .is_empty()
    );
}

#[tokio::test]
async fn keeps_a_key_and_value_at_the_maximum_lengths() {
    let key = "k".repeat(200);
    let value = "v".repeat(500);
    let reader = SourceReader {
        storage: StubStorage::with_bytes(row_with_attributes(vec![(key.clone(), value.clone())])),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window())
        .await
        .expect("ok");
    let metadata = &sample.executions.first().expect("one execution").metadata;
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata.first().expect("one entry").key, key);
    assert_eq!(metadata.first().expect("one entry").value, value);
}
