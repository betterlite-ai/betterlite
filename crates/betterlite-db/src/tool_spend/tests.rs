#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::tool_spend::ToolUsageTransaction;
use chrono::DateTime;
use rust_decimal::Decimal;

use super::index_rows::ToolIndexRow;
use super::rollups::{ToolDayRollup, daily_rollups};
use super::{
    FlushFuture, FlushPolicy, RetryPause, ToolSpendFailure, ToolSpendWriter,
    flush_tool_usage_transactions,
};
use crate::migration_ledger::lock::JitterSource;
use crate::spend_batches::StatementLimits;

mod fixtures;
mod postgres;

const PRE_SEND: FakeFailure = FakeFailure { pre_send: true };
const AFTER_SEND: FakeFailure = FakeFailure { pre_send: false };
const REQUEST_TIME: i64 = 1_767_323_045;

#[derive(Clone, Debug, thiserror::Error)]
#[error("fake tool spend failure, pre-send: {pre_send}")]
struct FakeFailure {
    pre_send: bool,
}

impl ToolSpendFailure for FakeFailure {
    fn is_pre_send(&self) -> bool {
        self.pre_send
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Index,
    Rollup,
}

#[derive(Default)]
struct FakeWriter {
    failures: Mutex<VecDeque<(Stage, FakeFailure)>>,
    index_writes: Mutex<Vec<Vec<(String, String)>>>,
    rollup_writes: Mutex<Vec<Vec<ToolDayRollup>>>,
}

impl FakeWriter {
    fn with_failures(failures: Vec<(Stage, FakeFailure)>) -> Self {
        Self {
            failures: Mutex::new(VecDeque::from(failures)),
            index_writes: Mutex::new(Vec::new()),
            rollup_writes: Mutex::new(Vec::new()),
        }
    }

    fn take_failure(&self, stage: Stage) -> Option<FakeFailure> {
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((expected, _)) = failures.front() else {
            return None;
        };
        if *expected != stage {
            return None;
        }
        failures.pop_front().map(|(_, failure)| failure)
    }

    fn record<T>(&self, writes: &Mutex<Vec<T>>, batch: T, stage: Stage) -> Result<(), FakeFailure> {
        writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(batch);
        match self.take_failure(stage) {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    fn index_calls(&self) -> Vec<Vec<(String, String)>> {
        self.index_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn rollup_calls(&self) -> Vec<Vec<ToolDayRollup>> {
        self.rollup_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ToolSpendWriter for FakeWriter {
    type Error = FakeFailure;

    fn write_index_rows<'writer>(
        &'writer self,
        rows: &'writer [ToolIndexRow],
    ) -> FlushFuture<'writer, Self::Error> {
        Box::pin(async move {
            let recorded = rows
                .iter()
                .map(|row| (row.request_id.clone(), row.tool_name.clone()))
                .collect();
            self.record(&self.index_writes, recorded, Stage::Index)
        })
    }

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ToolDayRollup],
    ) -> FlushFuture<'writer, Self::Error> {
        Box::pin(async move { self.record(&self.rollup_writes, rollups.to_vec(), Stage::Rollup) })
    }
}

#[derive(Default)]
struct RecordingPause {
    delays: Mutex<Vec<Duration>>,
}

impl RecordingPause {
    fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.delays
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(delay);
        })
    }
}

struct FixedJitter {
    nanos: u64,
}

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        self.nanos.min(upper_exclusive_nanos.saturating_sub(1))
    }
}

fn transaction(
    request_id: &str,
    date: &str,
    tool_names: &[&str],
    spend: Decimal,
    total_tokens: i64,
) -> ToolUsageTransaction {
    let start_time = DateTime::from_timestamp(REQUEST_TIME, 0).expect("a test instant");
    ToolUsageTransaction {
        request_id: request_id.to_owned(),
        date: date.to_owned(),
        start_time,
        tool_names: tool_names.iter().map(|name| (*name).to_owned()).collect(),
        spend,
        total_tokens,
    }
}

const fn policy(max_bytes: usize, max_rows: usize, retries: u32) -> FlushPolicy {
    FlushPolicy {
        limits: StatementLimits {
            max_bytes,
            max_rows,
        },
        retries,
    }
}

async fn flush_with(
    writer: &FakeWriter,
    transactions: &[ToolUsageTransaction],
    policy: FlushPolicy,
    nanos: u64,
) -> (Result<(), FakeFailure>, Vec<Duration>) {
    let jitter = FixedJitter { nanos };
    let pause = RecordingPause::default();
    let outcome =
        flush_tool_usage_transactions(writer, transactions, policy, &jitter, &pause).await;
    (outcome, pause.delays())
}

fn mixed_batch() -> [ToolUsageTransaction; 2] {
    [
        transaction(
            "req-1",
            "2026-01-02",
            &["search", "fetch"],
            Decimal::new(125, 2),
            42,
        ),
        transaction("req-2", "2026-01-02", &["fetch"], Decimal::new(75, 2), 8),
    ]
}

#[test]
fn daily_totals_add_money_exactly() {
    let batch = [
        transaction("req-1", "2026-01-02", &["search"], Decimal::new(1, 1), 3),
        transaction("req-2", "2026-01-02", &["search"], Decimal::new(2, 1), 5),
    ];
    assert_eq!(
        daily_rollups(&batch),
        [ToolDayRollup {
            date: String::from("2026-01-02"),
            tool_name: String::from("search"),
            spend: Decimal::new(3, 1),
            total_tokens: 8,
            request_count: 2,
        }]
    );
}

#[test]
fn a_tool_index_row_speaks_pythons_timestamp() {
    let whole_second = ToolIndexRow::new(
        String::from("req-1"),
        String::from("search"),
        DateTime::from_timestamp(REQUEST_TIME, 0).expect("a test instant"),
    );
    assert_eq!(
        serde_json::to_string(&whole_second).expect("the row serializes"),
        r#"{"request_id":"req-1","tool_name":"search","start_time":"2026-01-02 03:04:05+00:00"}"#
    );
    let fractional = ToolIndexRow::new(
        String::from("req-1"),
        String::from("search"),
        DateTime::from_timestamp(REQUEST_TIME, 123_456_000).expect("a test instant"),
    );
    assert_eq!(
        serde_json::to_string(&fractional).expect("the row serializes"),
        r#"{"request_id":"req-1","tool_name":"search","start_time":"2026-01-02 03:04:05.123456+00:00"}"#
    );
    assert_eq!(
        fractional.instant,
        DateTime::from_timestamp(REQUEST_TIME, 123_456_000).expect("a test instant")
    );
}

#[tokio::test]
async fn flushes_every_index_row_then_one_grouped_rollup_batch() {
    let writer = FakeWriter::default();
    let (outcome, delays) = flush_with(&writer, &mixed_batch(), policy(1_000, 100, 3), 0).await;
    assert!(outcome.is_ok());
    assert!(delays.is_empty());
    assert_eq!(
        writer.index_calls(),
        [[
            (String::from("req-1"), String::from("search")),
            (String::from("req-1"), String::from("fetch")),
            (String::from("req-2"), String::from("fetch")),
        ]]
    );
    assert_eq!(
        writer.rollup_calls(),
        [vec![
            ToolDayRollup {
                date: String::from("2026-01-02"),
                tool_name: String::from("fetch"),
                spend: Decimal::new(200, 2),
                total_tokens: 50,
                request_count: 2,
            },
            ToolDayRollup {
                date: String::from("2026-01-02"),
                tool_name: String::from("search"),
                spend: Decimal::new(125, 2),
                total_tokens: 42,
                request_count: 1,
            },
        ]]
    );
}

#[tokio::test]
async fn flushes_a_bounded_statement_per_slice() {
    let writer = FakeWriter::default();
    let batch = [transaction(
        "req-1",
        "2026-01-02",
        &["a", "b", "c"],
        Decimal::new(1, 0),
        1,
    )];
    let (outcome, _) = flush_with(&writer, &batch, policy(2_000_000, 2, 3), 0).await;
    assert!(outcome.is_ok());
    let lengths: Vec<usize> = writer.index_calls().iter().map(Vec::len).collect();
    assert_eq!(lengths, [2, 1]);
    let (by_bytes, _) = flush_with(&writer, &batch, policy(172, 100, 3), 0).await;
    assert!(by_bytes.is_ok());
    let split: Vec<usize> = writer.index_calls().iter().map(Vec::len).collect();
    assert_eq!(split, [2, 1, 2, 1]);
}

#[tokio::test]
async fn retries_a_pre_send_index_failure_and_rewrites_the_rows() {
    let writer = FakeWriter::with_failures(vec![(Stage::Index, PRE_SEND)]);
    let (outcome, delays) =
        flush_with(&writer, &mixed_batch(), policy(1_000, 100, 3), 250_000_000).await;
    assert!(outcome.is_ok());
    assert_eq!(delays, [Duration::from_millis(1_250)]);
    assert_eq!(writer.index_calls().len(), 2);
    assert_eq!(writer.rollup_calls().len(), 1);
}

#[tokio::test]
async fn retries_a_pre_send_rollup_failure_with_the_whole_attempt() {
    let writer = FakeWriter::with_failures(vec![(Stage::Rollup, PRE_SEND)]);
    let (outcome, delays) = flush_with(&writer, &mixed_batch(), policy(1_000, 100, 3), 0).await;
    assert!(outcome.is_ok());
    assert_eq!(delays, [Duration::from_secs(1)]);
    assert_eq!(writer.index_calls().len(), 2);
    assert_eq!(writer.rollup_calls().len(), 2);
}

#[tokio::test]
async fn stops_after_the_retry_limit() {
    let writer = FakeWriter::with_failures(vec![(Stage::Index, PRE_SEND); 5]);
    let (outcome, delays) = flush_with(&writer, &mixed_batch(), policy(1_000, 100, 3), 0).await;
    assert!(outcome.is_err());
    assert_eq!(writer.index_calls().len(), 4);
    assert_eq!(
        delays,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
}

#[tokio::test]
async fn does_not_retry_when_the_retry_limit_is_zero() {
    let writer = FakeWriter::with_failures(vec![(Stage::Index, PRE_SEND)]);
    let (outcome, delays) = flush_with(&writer, &mixed_batch(), policy(1_000, 100, 0), 0).await;
    assert!(outcome.is_err());
    assert!(delays.is_empty());
    assert_eq!(writer.index_calls().len(), 1);
}

#[tokio::test]
async fn does_not_retry_an_ambiguous_write() {
    let writer = FakeWriter::with_failures(vec![(Stage::Rollup, AFTER_SEND)]);
    let (outcome, delays) = flush_with(&writer, &mixed_batch(), policy(1_000, 100, 3), 0).await;
    assert!(outcome.is_err());
    assert!(delays.is_empty());
    assert_eq!(writer.index_calls().len(), 1);
    assert_eq!(writer.rollup_calls().len(), 1);
}

#[tokio::test]
async fn an_empty_batch_touches_nothing() {
    let writer = FakeWriter::default();
    let (outcome, delays) = flush_with(&writer, &[], policy(1_000, 100, 3), 0).await;
    assert!(outcome.is_ok());
    assert!(delays.is_empty());
    assert!(writer.index_calls().is_empty());
    assert!(writer.rollup_calls().is_empty());
}
