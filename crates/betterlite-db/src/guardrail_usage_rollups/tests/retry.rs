use std::time::Duration;

use rust_decimal::Decimal;

use crate::guardrail_usage_rollups::{
    GuardrailCounts, MetricsKey, RollupRow, UsageUnitIncrement, UsageUnitKey, flush,
};

use super::support::{FakeSink, NOT_RETRY_SAFE, PauseLog, RETRY_SAFE, capture_logs};

fn metric_row(guardrail_id: &str) -> RollupRow {
    RollupRow::Metrics {
        key: MetricsKey {
            guardrail_id: String::from(guardrail_id),
            date: String::from("2026-01-02"),
        },
        counts: GuardrailCounts {
            requests_evaluated: 1,
            passed_count: 1,
            blocked_count: 0,
            flagged_count: 0,
        },
    }
}

fn unit_row(guardrail_id: &str, usage_unit: &str) -> RollupRow {
    RollupRow::Units {
        key: UsageUnitKey {
            guardrail_id: String::from(guardrail_id),
            date: String::from("2026-01-02"),
            team_id: String::new(),
            api_key: String::new(),
            usage_unit: String::from(usage_unit),
        },
        increment: UsageUnitIncrement {
            units: 1,
            cost: Decimal::ZERO,
            untracked_units: 1,
        },
    }
}

async fn flush_with(sink: &FakeSink, rows: Vec<RollupRow>) -> (Vec<RollupRow>, Vec<Duration>) {
    let pause = PauseLog::default();
    let failed = flush::rows(sink, rows, &pause).await;
    (failed, pause.waits())
}

fn written(sink: &FakeSink) -> Vec<String> {
    sink.metric_calls()
        .into_iter()
        .map(|(key, _)| key.guardrail_id)
        .collect()
}

#[tokio::test]
async fn writes_a_row_once_without_waiting() {
    let sink = FakeSink::default();
    let (failed, waits) = flush_with(&sink, vec![metric_row("g1")]).await;
    assert!(failed.is_empty());
    assert!(waits.is_empty());
    assert_eq!(written(&sink), ["g1"]);
}

#[tokio::test]
async fn an_empty_row_list_writes_nothing() {
    let sink = FakeSink::default();
    let (failed, waits) = flush_with(&sink, Vec::new()).await;
    assert!(failed.is_empty());
    assert!(waits.is_empty());
    assert!(written(&sink).is_empty());
}

#[tokio::test]
async fn retries_a_retry_safe_failure_after_one_second() {
    let sink = FakeSink::failing_upserts(vec![Some(RETRY_SAFE)]);
    let (failed, waits) = flush_with(&sink, vec![metric_row("g1")]).await;
    assert!(failed.is_empty());
    assert_eq!(waits, [Duration::from_secs(1)]);
    assert_eq!(written(&sink), ["g1", "g1"]);
}

#[tokio::test]
async fn doubles_the_wait_for_each_retry() {
    let sink = FakeSink::failing_upserts(vec![Some(RETRY_SAFE); 3]);
    let (failed, waits) = flush_with(&sink, vec![metric_row("g1")]).await;
    assert!(failed.is_empty());
    assert_eq!(
        waits,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
    assert_eq!(written(&sink), ["g1", "g1", "g1", "g1"]);
}

#[tokio::test]
async fn requeues_a_row_that_still_fails_after_three_retries() {
    let sink = FakeSink::failing_upserts(vec![Some(RETRY_SAFE); 5]);
    let (failed, waits) = flush_with(&sink, vec![metric_row("g1")]).await;
    assert_eq!(failed, [metric_row("g1")]);
    assert_eq!(
        waits,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
    assert_eq!(written(&sink).len(), 4);
}

#[tokio::test]
async fn drops_a_row_that_fails_without_a_retry_safe_error() {
    let capture = capture_logs();
    let sink = FakeSink::failing_upserts(vec![Some(NOT_RETRY_SAFE)]);
    let (failed, waits) = flush_with(&sink, vec![metric_row("g1")]).await;
    assert!(failed.is_empty());
    assert!(waits.is_empty());
    assert_eq!(written(&sink), ["g1"]);
    assert!(capture.logs.has("WARN", "not safe to retry"));
}

#[tokio::test]
async fn names_the_row_type_that_failed_in_the_warning() {
    let capture = capture_logs();
    let sink = FakeSink::failing_upserts(vec![Some(NOT_RETRY_SAFE), Some(NOT_RETRY_SAFE)]);
    let rows = vec![metric_row("g1"), unit_row("g1", "a")];
    let (failed, waits) = flush_with(&sink, rows).await;
    assert!(failed.is_empty());
    assert!(waits.is_empty());
    assert!(capture.logs.has("WARN", "daily metrics upsert failed"));
    assert!(capture.logs.has("WARN", "usage unit upsert failed"));
}

#[tokio::test]
async fn keeps_writing_the_other_rows_while_one_retries() {
    let sink = FakeSink::failing_upserts(vec![Some(RETRY_SAFE), None]);
    let rows = vec![metric_row("g1"), metric_row("g2")];
    let (failed, waits) = flush_with(&sink, rows).await;
    assert!(failed.is_empty());
    assert_eq!(waits, [Duration::from_secs(1)]);
    assert_eq!(written(&sink), ["g1", "g2", "g1"]);
}

#[tokio::test]
async fn requeues_only_the_rows_that_kept_failing() {
    let sink = FakeSink::failing_upserts(vec![
        Some(RETRY_SAFE),
        None,
        Some(RETRY_SAFE),
        Some(RETRY_SAFE),
        Some(RETRY_SAFE),
    ]);
    let rows = vec![metric_row("g1"), metric_row("g2")];
    let (failed, waits) = flush_with(&sink, rows).await;
    assert_eq!(failed, [metric_row("g1")]);
    assert_eq!(
        waits,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
    assert_eq!(written(&sink), ["g1", "g2", "g1", "g1", "g1"]);
}
