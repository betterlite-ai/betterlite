use std::collections::VecDeque;
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use rust_decimal::Decimal;

use crate::guardrail_usage_rollups::pending::PendingRollups;
use crate::guardrail_usage_rollups::{
    GuardrailCounts, GuardrailIndexRow, GuardrailRollupFailure, GuardrailRollupSink,
    GuardrailSpendLog, MetricsKey, RollupFuture, SharedPendingRollups, UsageUnitIncrement,
    UsageUnitKey,
};
use crate::tool_spend::RetryPause;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct UnitCall {
    pub(super) guardrail_id: String,
    pub(super) date: String,
    pub(super) team_id: String,
    pub(super) api_key: String,
    pub(super) usage_unit: String,
    pub(super) units: i64,
    pub(super) cost: Decimal,
    pub(super) untracked_units: i64,
}

impl UnitCall {
    pub(super) fn new(key: &UsageUnitKey, increment: UsageUnitIncrement) -> Self {
        Self {
            guardrail_id: key.guardrail_id.clone(),
            date: key.date.clone(),
            team_id: key.team_id.clone(),
            api_key: key.api_key.clone(),
            usage_unit: key.usage_unit.clone(),
            units: increment.units,
            cost: increment.cost,
            untracked_units: increment.untracked_units,
        }
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn from_json(text: &str) -> GuardrailSpendLog {
    serde_json::from_str(text).expect("a spend log payload")
}

pub(super) fn shared() -> SharedPendingRollups {
    Arc::new(Mutex::new(PendingRollups::default()))
}

pub(super) const DATE: &str = "2026-01-02";

pub(super) fn metrics_key(guardrail_id: &str) -> MetricsKey {
    MetricsKey {
        guardrail_id: String::from(guardrail_id),
        date: String::from(DATE),
    }
}

pub(super) fn unit_key(guardrail_id: &str, usage_unit: &str) -> UsageUnitKey {
    UsageUnitKey {
        guardrail_id: String::from(guardrail_id),
        date: String::from(DATE),
        team_id: String::from("team-1"),
        api_key: String::from("sk-1"),
        usage_unit: String::from(usage_unit),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("fake rollup failure, retry safe: {retry_safe}")]
pub(super) struct FakeFailure {
    pub(super) retry_safe: bool,
}

pub(super) const RETRY_SAFE: FakeFailure = FakeFailure { retry_safe: true };
pub(super) const NOT_RETRY_SAFE: FakeFailure = FakeFailure { retry_safe: false };

impl GuardrailRollupFailure for FakeFailure {
    fn is_retry_safe(&self) -> bool {
        self.retry_safe
    }
}

#[derive(Default)]
pub(super) struct FakeSink {
    index_calls: Mutex<Vec<Vec<GuardrailIndexRow>>>,
    metric_calls: Mutex<Vec<(MetricsKey, GuardrailCounts)>>,
    unit_calls: Mutex<Vec<(UsageUnitKey, UsageUnitIncrement)>>,
    upsert_failures: Mutex<VecDeque<Option<FakeFailure>>>,
    index_failures: Mutex<VecDeque<FakeFailure>>,
}

impl FakeSink {
    fn with_failures(
        upsert_failures: Vec<Option<FakeFailure>>,
        index_failures: Vec<FakeFailure>,
    ) -> Self {
        Self {
            index_calls: Mutex::new(Vec::new()),
            metric_calls: Mutex::new(Vec::new()),
            unit_calls: Mutex::new(Vec::new()),
            upsert_failures: Mutex::new(VecDeque::from(upsert_failures)),
            index_failures: Mutex::new(VecDeque::from(index_failures)),
        }
    }

    pub(super) fn failing_upserts(failures: Vec<Option<FakeFailure>>) -> Self {
        Self::with_failures(failures, Vec::new())
    }

    pub(super) fn failing_index(failures: Vec<FakeFailure>) -> Self {
        Self::with_failures(Vec::new(), failures)
    }

    pub(super) fn index_calls(&self) -> Vec<Vec<GuardrailIndexRow>> {
        lock(&self.index_calls).clone()
    }

    pub(super) fn metric_calls(&self) -> Vec<(MetricsKey, GuardrailCounts)> {
        lock(&self.metric_calls).clone()
    }

    pub(super) fn unit_calls(&self) -> Vec<UnitCall> {
        lock(&self.unit_calls)
            .iter()
            .map(|(key, increment)| UnitCall::new(key, *increment))
            .collect()
    }

    fn next_upsert_failure(&self) -> Option<FakeFailure> {
        lock(&self.upsert_failures).pop_front().flatten()
    }

    fn record_metric(&self, key: &MetricsKey, counts: GuardrailCounts) -> Result<(), FakeFailure> {
        lock(&self.metric_calls).push((key.clone(), counts));
        self.next_upsert_failure().map_or(Ok(()), Err)
    }

    fn record_unit(
        &self,
        key: &UsageUnitKey,
        increment: &UsageUnitIncrement,
    ) -> Result<(), FakeFailure> {
        lock(&self.unit_calls).push((key.clone(), *increment));
        self.next_upsert_failure().map_or(Ok(()), Err)
    }
}

impl GuardrailRollupSink for FakeSink {
    type Error = FakeFailure;

    fn create_index_rows<'rows>(
        &'rows self,
        rows: &'rows [GuardrailIndexRow],
    ) -> RollupFuture<'rows, FakeFailure> {
        Box::pin(async move {
            lock(&self.index_calls).push(rows.to_vec());
            match lock(&self.index_failures).pop_front() {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        })
    }

    fn upsert_metrics<'row>(
        &'row self,
        key: &'row MetricsKey,
        counts: GuardrailCounts,
    ) -> RollupFuture<'row, FakeFailure> {
        Box::pin(async move { self.record_metric(key, counts) })
    }

    fn upsert_units<'row>(
        &'row self,
        key: &'row UsageUnitKey,
        increment: &'row UsageUnitIncrement,
    ) -> RollupFuture<'row, FakeFailure> {
        Box::pin(async move { self.record_unit(key, increment) })
    }
}

#[derive(Default)]
pub(super) struct PauseLog {
    waits: Mutex<Vec<Duration>>,
}

impl PauseLog {
    pub(super) fn waits(&self) -> Vec<Duration> {
        lock(&self.waits).clone()
    }
}

impl RetryPause for PauseLog {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            lock(&self.waits).push(delay);
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(super) fn has(&self, level: &str, needle: &str) -> bool {
        let bytes = lock(&self.0).clone();
        let text = String::from_utf8(bytes).expect("the log writer records UTF-8");
        text.lines()
            .any(|line| line.contains(level) && line.contains(needle))
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buffer: &[u8]) -> IoResult<usize> {
        lock(&self.0).extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) struct LogCapture {
    pub(super) logs: CapturedLogs,
    _guard: DefaultGuard,
}

pub(super) fn capture_logs() -> LogCapture {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    let guard = set_default(subscriber);
    LogCapture {
        logs,
        _guard: guard,
    }
}
