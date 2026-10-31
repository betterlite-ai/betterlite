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

fn rows(count: usize, eligible: i64, selected: i64) -> Vec<u8> {
    let items: Vec<ExecutionRow> = (0..count)
        .map(|index| ExecutionRow {
            selection_key: format!("key-{index}"),
            source: ExecutionSource::Traces,
            trace_id: format!("t{index}"),
            trace_ref: String::new(),
            team_id: "team-a".to_owned(),
            name: "run".to_owned(),
            start_time: "2026-01-01T00:00:00Z".to_owned(),
            span_count: 1,
            root_seen: 1,
            eligible,
            selected,
            service: String::new(),
            attributes: vec![],
        })
        .collect();
    serde_json::to_vec(&items).expect("encodes")
}

fn window(offset: i64, page_size: usize, preview: bool) -> SampleWindow {
    SampleWindow {
        start: 0,
        end: 1000,
        offset,
        page_size,
        preview,
        cursor: String::new(),
    }
}

#[tokio::test]
async fn a_full_page_sets_next_cursor_to_the_last_selection_key() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows(100, 500, 500)),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window(0, 100, false))
        .await
        .expect("ok");
    assert_eq!(sample.next_cursor, Some("key-99".to_owned()));
}

#[tokio::test]
async fn a_partial_page_has_no_next_cursor() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows(5, 5, 5)),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window(0, 100, false))
        .await
        .expect("ok");
    assert_eq!(sample.next_cursor, None);
}

#[tokio::test]
async fn next_offset_advances_when_selected_continues_past_the_page() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows(100, 500, 500)),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window(0, 100, false))
        .await
        .expect("ok");
    assert_eq!(sample.next_offset, Some(100));
}

#[tokio::test]
async fn next_offset_is_none_once_the_selected_boundary_is_reached() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows(100, 500, 100)),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window(0, 100, false))
        .await
        .expect("ok");
    assert_eq!(sample.next_offset, None);
}

#[tokio::test]
async fn preview_mode_compares_against_eligible_not_selected() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows(100, 500, 10)),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window(0, 100, true))
        .await
        .expect("ok");
    assert_eq!(sample.next_offset, Some(100));
}

#[tokio::test]
async fn an_empty_page_has_no_next_offset() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(rows(0, 0, 0)),
    };
    let sample = reader
        .sample(&Scope::default(), &settings(), window(0, 100, false))
        .await
        .expect("ok");
    assert_eq!(sample.next_offset, None);
}
