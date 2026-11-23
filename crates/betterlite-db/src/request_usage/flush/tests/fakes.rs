use std::collections::VecDeque;
use std::future::{Future, pending, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::autorouter_drain::{TurnFuture, TurnWriter};
use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::baseline::records::BaselineAccountingRecord;
use crate::baseline::store::error::StoreError;
use crate::baseline::store::{AppendResult, BaselineAccounting, ProjectResult};
use crate::guardrail_usage_rollups::{
    GuardrailCounts, GuardrailIndexRow, GuardrailRollupFailure, GuardrailRollupSink, MetricsKey,
    RollupFuture, UsageUnitIncrement, UsageUnitKey,
};
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::lock::JitterSource;
use crate::model_usage::{ModelUsageFailure, ModelUsageFuture, ModelUsageRollup, ModelUsageWriter};
use crate::redis_spend::log_row::SpendLogRow;
use crate::shadow_tables::rows::{
    FunnelCounters, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow, ShadowTables,
};
use crate::spend_log_storage::rows::writer::{SpendUpdateClient, SpendUpdateRequest};
use crate::spend_log_storage::{SpendLogFuture, SpendLogWriteError, SpendLogsWriter};
use crate::tool_spend::index_rows::ToolIndexRow;
use crate::tool_spend::rollups::ToolDayRollup;
use crate::tool_spend::{FlushFuture, RetryPause, ToolSpendFailure, ToolSpendWriter};

use super::support::Trace;

// Every fake keeps what it was handed behind one lock, so a poisoned lock still yields its rows.
fn append<T: Clone>(store: &Mutex<Vec<T>>, items: &[T]) {
    store
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .extend_from_slice(items);
}

// A scripted spend log write failure, each flag set the way a native error would report itself.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the scripted spend log insert failed")]
pub(super) struct ScriptedFailure {
    flags: u8,
}

const DATA_REJECTION: u8 = 1;
const SERVICE_UNAVAILABLE: u8 = 2;
const DEADLOCK: u8 = 4;
const DATABASE: u8 = 8;

impl ScriptedFailure {
    pub(super) const DATABASE_ERROR: Self = Self { flags: DATABASE };

    const fn set(self, flag: u8) -> bool {
        (self.flags & flag) != 0
    }
}

impl SpendLogWriteError for ScriptedFailure {
    fn is_transient(&self) -> bool {
        self.set(SERVICE_UNAVAILABLE) || self.set(DEADLOCK)
    }

    fn is_data_rejection(&self) -> bool {
        self.set(DATA_REJECTION)
    }

    fn is_service_unavailable(&self) -> bool {
        self.set(SERVICE_UNAVAILABLE)
    }

    fn is_deadlock(&self) -> bool {
        self.set(DEADLOCK)
    }

    fn is_database(&self) -> bool {
        self.set(DATABASE)
    }
}

// One spend log write attempt: it records the batch it wrote, or hands back the next scripted failure.
pub(super) struct FakeLogWriter {
    pub(super) trace: Trace,
    failures: Mutex<VecDeque<ScriptedFailure>>,
    rows: Mutex<Vec<SpendLogRow>>,
    calls: AtomicUsize,
    stalled: AtomicBool,
}

impl FakeLogWriter {
    pub(super) fn new(trace: Trace) -> Self {
        Self {
            trace,
            failures: Mutex::new(VecDeque::new()),
            rows: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            stalled: AtomicBool::new(false),
        }
    }

    // A write that never settles, so a test can drop the pass at its await.
    pub(super) fn stall(&self) {
        self.stalled.store(true, Ordering::SeqCst);
    }

    pub(super) fn fail_next(&self, failure: ScriptedFailure) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(failure);
    }

    pub(super) fn rows(&self) -> Vec<SpendLogRow> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl SpendLogsWriter for FakeLogWriter {
    type Error = ScriptedFailure;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, Self::Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.trace.record("log");
            if self.stalled.load(Ordering::SeqCst) {
                pending::<()>().await;
            }
            let failure = self
                .failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            if let Some(scripted) = failure {
                return Err(scripted);
            }
            append(&self.rows, rows);
            Ok(rows.len())
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the external spend writer is unreachable")]
pub(super) struct NoHttpFailure;

// The external writer a job is never attached to.
pub(super) struct NoClient;

impl SpendUpdateClient for NoClient {
    type Error = NoHttpFailure;

    fn post<'call>(
        &'call self,
        _request: SpendUpdateRequest<'call>,
    ) -> SpendLogFuture<'call, Result<u16, NoHttpFailure>> {
        Box::pin(async { Err(NoHttpFailure) })
    }
}

// RetryPause that returns at once, so a retry round costs the test no time.
pub(super) struct RecordingPause;

impl RetryPause for RecordingPause {
    fn pause(&self, _delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(ready(()))
    }
}

// WriteClock whose second instant is five seconds after its first, so a reported duration is exact.
pub(super) struct SteppingClock {
    start: Instant,
    steps: AtomicU32,
}

impl SteppingClock {
    pub(super) fn new() -> Self {
        Self {
            start: Instant::now(),
            steps: AtomicU32::new(0),
        }
    }
}

impl WriteClock for SteppingClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn instant(&self) -> Instant {
        let step = self.steps.fetch_add(1, Ordering::SeqCst);
        self.start
            .checked_add(Duration::from_secs(u64::from(step).saturating_mul(5)))
            .expect("an instant after the first step")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted rollup write failed")]
pub(super) struct RollupFailure;

impl GuardrailRollupFailure for RollupFailure {
    fn is_retry_safe(&self) -> bool {
        false
    }
}

// The guardrail sink records what it was asked to write, and fails every call on request.
pub(super) struct FakeGuardrailSink {
    pub(super) trace: Trace,
    rows: Mutex<Vec<GuardrailIndexRow>>,
    fail: AtomicBool,
}

impl FakeGuardrailSink {
    pub(super) fn new(trace: Trace) -> Self {
        Self {
            trace,
            rows: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
        }
    }

    pub(super) fn fail(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }

    pub(super) fn rows(&self) -> Vec<GuardrailIndexRow> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn failed(&self) -> bool {
        self.fail.load(Ordering::SeqCst)
    }

    fn check(&self) -> Result<(), RollupFailure> {
        self.trace.record("guardrail");
        if self.failed() {
            return Err(RollupFailure);
        }
        Ok(())
    }
}

impl GuardrailRollupSink for FakeGuardrailSink {
    type Error = RollupFailure;

    fn create_index_rows<'rows>(
        &'rows self,
        rows: &'rows [GuardrailIndexRow],
    ) -> RollupFuture<'rows, RollupFailure> {
        Box::pin(async move {
            self.check()?;
            append(&self.rows, rows);
            Ok(())
        })
    }

    fn upsert_metrics<'row>(
        &'row self,
        _key: &'row MetricsKey,
        _counts: GuardrailCounts,
    ) -> RollupFuture<'row, RollupFailure> {
        Box::pin(ready(self.check()))
    }

    fn upsert_units<'row>(
        &'row self,
        _key: &'row UsageUnitKey,
        _increment: &'row UsageUnitIncrement,
    ) -> RollupFuture<'row, RollupFailure> {
        Box::pin(ready(self.check()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted tool spend write failed")]
pub(super) struct ToolFailure;

impl ToolSpendFailure for ToolFailure {
    fn is_pre_send(&self) -> bool {
        false
    }
}

// The tool writer counts index rows and keeps the rollups it was handed.
#[derive(Default)]
pub(super) struct FakeToolWriter {
    pub(super) trace: Trace,
    index_rows: AtomicUsize,
    rollups: Mutex<Vec<ToolDayRollup>>,
    fail: AtomicBool,
}

impl FakeToolWriter {
    pub(super) fn new(trace: Trace) -> Self {
        let writer = Self::default();
        Self { trace, ..writer }
    }

    pub(super) fn fail(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }

    pub(super) fn index_rows(&self) -> usize {
        self.index_rows.load(Ordering::SeqCst)
    }

    pub(super) fn rollups(&self) -> Vec<ToolDayRollup> {
        self.rollups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn failed(&self) -> bool {
        self.fail.load(Ordering::SeqCst)
    }

    fn check(&self) -> Result<(), ToolFailure> {
        self.trace.record("tool");
        if self.failed() {
            return Err(ToolFailure);
        }
        Ok(())
    }

    fn count_rows(&self, rows: &[ToolIndexRow]) -> Result<(), ToolFailure> {
        self.check()?;
        self.index_rows.fetch_add(rows.len(), Ordering::SeqCst);
        Ok(())
    }

    fn store_rollups(&self, rollups: &[ToolDayRollup]) -> Result<(), ToolFailure> {
        self.check()?;
        append(&self.rollups, rollups);
        Ok(())
    }
}

impl ToolSpendWriter for FakeToolWriter {
    type Error = ToolFailure;

    fn write_index_rows<'writer>(
        &'writer self,
        rows: &'writer [ToolIndexRow],
    ) -> FlushFuture<'writer, ToolFailure> {
        Box::pin(async move { self.count_rows(rows) })
    }

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ToolDayRollup],
    ) -> FlushFuture<'writer, ToolFailure> {
        Box::pin(async move { self.store_rollups(rollups) })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted model usage write failed")]
pub(super) struct ModelFailure;

impl ModelUsageFailure for ModelFailure {
    fn is_pre_send(&self) -> bool {
        false
    }
}

// The model writer keeps the rollups it was handed.
pub(super) struct FakeModelWriter {
    pub(super) trace: Trace,
    rollups: Mutex<Vec<ModelUsageRollup>>,
    fail: AtomicBool,
}

impl FakeModelWriter {
    pub(super) fn new(trace: Trace) -> Self {
        Self {
            trace,
            rollups: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
        }
    }

    pub(super) fn fail(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }

    pub(super) fn rollups(&self) -> Vec<ModelUsageRollup> {
        self.rollups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn failed(&self) -> bool {
        self.fail.load(Ordering::SeqCst)
    }

    fn check(&self) -> Result<(), ModelFailure> {
        self.trace.record("model");
        if self.failed() {
            return Err(ModelFailure);
        }
        Ok(())
    }

    fn store_rollups(&self, rollups: &[ModelUsageRollup]) -> Result<(), ModelFailure> {
        self.check()?;
        append(&self.rollups, rollups);
        Ok(())
    }
}

impl ModelUsageWriter for FakeModelWriter {
    type Error = ModelFailure;

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ModelUsageRollup],
    ) -> ModelUsageFuture<'writer, ModelFailure> {
        Box::pin(async move { self.store_rollups(rollups) })
    }
}

// The turn drain writer keeps every turn it was asked to write.
pub(super) struct FakeTurnWriter {
    pub(super) trace: Trace,
    turns: Mutex<Vec<AutoRouterTurnTransaction>>,
}

impl FakeTurnWriter {
    pub(super) fn new(trace: Trace) -> Self {
        Self {
            trace,
            turns: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn turns(&self) -> Vec<AutoRouterTurnTransaction> {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TurnWriter for FakeTurnWriter {
    fn write_turn<'writer>(
        &'writer self,
        turn: &'writer AutoRouterTurnTransaction,
        _user_scoped: bool,
    ) -> TurnFuture<'writer> {
        Box::pin(async move {
            self.trace.record("turn");
            self.turns
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(turn.clone());
            Ok(())
        })
    }
}

// The accounting store records what it appended and claims one dirty scope per pass.
pub(super) struct FakeAccounting {
    pub(super) trace: Trace,
    appended: Mutex<Vec<BaselineAccountingRecord>>,
    dirty: Mutex<Vec<String>>,
}

impl FakeAccounting {
    pub(super) fn new(trace: Trace) -> Self {
        Self {
            trace,
            appended: Mutex::new(Vec::new()),
            dirty: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn mark_dirty(&self, scope: &str) {
        self.dirty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scope.to_owned());
    }

    pub(super) fn appended(&self) -> usize {
        self.appended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl BaselineAccounting for FakeAccounting {
    fn append(
        &self,
        record: &BaselineAccountingRecord,
    ) -> impl Future<Output = AppendResult> + Send {
        self.trace.record("baseline");
        self.appended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        ready(AppendResult::Recorded)
    }

    fn project(&self, _scope: &str) -> impl Future<Output = ProjectResult> + Send {
        self.trace.record("baseline");
        ready(ProjectResult::Published)
    }

    fn claim_dirty(&self) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send {
        self.trace.record("baseline");
        let scopes = self
            .dirty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        ready(Ok(scopes))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted shadow table write failed")]
pub(super) struct ShadowFailure;

// The shadow store keeps the funnel counters it was asked to upsert.
pub(super) struct FakeShadowStore {
    pub(super) trace: Trace,
    funnel: Mutex<Vec<(String, FunnelCounters)>>,
}

impl FakeShadowStore {
    pub(super) fn new(trace: Trace) -> Self {
        Self {
            trace,
            funnel: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn funnel(&self) -> Vec<(String, FunnelCounters)> {
        self.funnel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ShadowTables for FakeShadowStore {
    type Error = ShadowFailure;

    fn find_active_jobs(
        &self,
        _now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, ShadowFailure> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn attempt_stats<'database>(
        &'database self,
        _job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, ShadowFailure> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn insert_attempt<'database>(
        &'database self,
        _attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), ShadowFailure> {
        Box::pin(ready(Ok(())))
    }

    fn upsert_funnel<'database>(
        &'database self,
        job_id: &'database str,
        counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), ShadowFailure> {
        self.trace.record("funnel");
        self.funnel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((job_id.to_owned(), counters));
        Box::pin(ready(Ok(())))
    }
}

// No jitter, so a retry delay is exactly the base delay.
pub(super) struct NoJitter;

impl JitterSource for NoJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}
