#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio::sync::Notify;
use tokio::time::Instant as TokioInstant;

use crate::budget_reset_policy::cache_invalidation::{CacheDeleteFuture, ResetCache};
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::entity_spend_commit::EntityTableWriter;
use crate::entity_spend_commit::TableFuture;
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::lock::JitterSource;
use crate::redis_spend::lists::{
    BufferPop, BufferReply, SpendBufferListError, SpendBufferLists, SpendListFuture,
};
use crate::spend_commit::daily::{ChunkFuture, DailyChunkFailure, DailyChunkWriter};
use crate::spend_commit::phase::{
    CommitPhase, CommitPhaseError, CommitPhaseState, CommitTransition,
};
use crate::spend_commit::policy::{SpendFailure, SpendUpdateFailure};
use crate::spend_commit::tables::SpendTable;
use crate::spend_flush::discovery::{DiscoveryFuture, ToolDiscoveryWriter};
use crate::spend_flush::service::{ServiceFuture, SpendLogsService};
use crate::spend_flush::window::{WindowCommitFuture, WindowSpendCommit};
use crate::tool_discovery::ToolDiscoveryItem;
use crate::tool_spend::RetryPause;
use crate::window_queue::WindowSpendTransaction;

// One ordered log every fake appends to, so a test reads what reached which port and in what order.
#[derive(Default)]
pub(crate) struct Events {
    entries: Mutex<Vec<String>>,
}

impl Events {
    pub(crate) fn push(&self, entry: String) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct FixedClock {
    pub(crate) instant: Instant,
}

impl WriteClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH
    }

    fn instant(&self) -> Instant {
        self.instant
    }
}

pub(crate) struct FixedJitter;

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

// Retry waits are recorded and skipped.
#[derive(Default)]
pub(crate) struct PauseLog {
    delays: Mutex<Vec<Duration>>,
}

impl RetryPause for PauseLog {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
        Box::pin(ready(()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake entity table failed")]
pub(crate) struct FakeEntityError {
    pub(crate) failure: SpendFailure,
}

impl SpendUpdateFailure for FakeEntityError {
    fn spend_failure(&self) -> SpendFailure {
        self.failure
    }
}

pub(crate) struct FakeEntityWriter {
    events: Arc<Events>,
    failure: Mutex<Option<SpendFailure>>,
}

impl FakeEntityWriter {
    pub(crate) fn new(events: Arc<Events>) -> Self {
        Self {
            events,
            failure: Mutex::new(None),
        }
    }

    pub(crate) fn fail_with(&self, failure: SpendFailure) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
    }
}

impl EntityTableWriter for FakeEntityWriter {
    type Error = FakeEntityError;

    fn write_table<'writer>(
        &'writer self,
        table: SpendTable,
        increments: &'writer BTreeMap<String, Decimal>,
    ) -> TableFuture<'writer, FakeEntityError> {
        self.events
            .push(format!("entity {} {}", table.name(), increments.len()));
        let failure = *self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        Box::pin(ready(match failure {
            Some(kind) => Err(FakeEntityError { failure: kind }),
            None => Ok(()),
        }))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake cache is down")]
pub(crate) struct FakeCacheError;

pub(crate) struct FakeCache {
    events: Arc<Events>,
}

impl FakeCache {
    pub(crate) fn new(events: Arc<Events>) -> Self {
        Self { events }
    }
}

impl ResetCache for FakeCache {
    type Failure = FakeCacheError;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, FakeCacheError> {
        self.events.push(format!("counters {}", keys.join(",")));
        Box::pin(ready(Ok(())))
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, FakeCacheError> {
        self.events.push(format!("evict {}", keys.join(",")));
        Box::pin(ready(Ok(())))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FakeDailyError {
    #[error("the fake daily chunk failed")]
    Failed { failure: SpendFailure },
    #[error("the fake daily chunk was cancelled")]
    Cancelled,
}

impl From<CommitPhaseError> for FakeDailyError {
    fn from(_error: CommitPhaseError) -> Self {
        Self::Cancelled
    }
}

impl SpendUpdateFailure for FakeDailyError {
    fn spend_failure(&self) -> SpendFailure {
        match self {
            Self::Failed { failure } => *failure,
            Self::Cancelled => SpendFailure::Other,
        }
    }
}

impl DailyChunkFailure for FakeDailyError {
    fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

// What one chunk of the fake daily writer does. Wait shapes stand still until the test releases them, so a shutdown can arrive in the middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkStep {
    Commit,
    Fail(SpendFailure),
    WaitThenCommit,
    BeginThenWaitThenCommit,
    BeginThenWaitThenFail(SpendFailure),
}

pub(crate) struct FakeDailyWriter {
    events: Arc<Events>,
    script: Mutex<VecDeque<ChunkStep>>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl FakeDailyWriter {
    pub(crate) fn new(events: Arc<Events>, steps: Vec<ChunkStep>) -> Self {
        Self {
            events,
            script: Mutex::new(VecDeque::from(steps)),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }

    // Wake the test once this chunk is standing still, and let it go on the test's signal.
    pub(crate) fn entered(&self) -> Arc<Notify> {
        Arc::clone(&self.entered)
    }

    pub(crate) fn release(&self) -> Arc<Notify> {
        Arc::clone(&self.release)
    }

    fn next_step(&self) -> ChunkStep {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ChunkStep::Commit)
    }

    async fn await_release(&self) {
        self.entered.notify_one();
        self.release.notified().await;
    }

    // A phase that refuses the transition has already been cancelled by the shutdown, which is how the real statement ends.
    async fn run_step(&self, step: ChunkStep, phase: &CommitPhase) -> Result<(), FakeDailyError> {
        match step {
            ChunkStep::Commit => {
                phase.apply(CommitTransition::Begin)?;
                phase.apply(CommitTransition::Finish)?;
                Ok(())
            }
            ChunkStep::Fail(failure) => Err(FakeDailyError::Failed { failure }),
            ChunkStep::WaitThenCommit => {
                self.await_release().await;
                if phase.state() == CommitPhaseState::Cancelled {
                    return Err(FakeDailyError::Cancelled);
                }
                phase.apply(CommitTransition::Begin)?;
                phase.apply(CommitTransition::Finish)?;
                Ok(())
            }
            ChunkStep::BeginThenWaitThenCommit => {
                phase.apply(CommitTransition::Begin)?;
                self.await_release().await;
                phase.apply(CommitTransition::Finish)?;
                Ok(())
            }
            ChunkStep::BeginThenWaitThenFail(failure) => {
                phase.apply(CommitTransition::Begin)?;
                self.await_release().await;
                Err(FakeDailyError::Failed { failure })
            }
        }
    }
}

impl DailyChunkWriter for FakeDailyWriter {
    type Error = FakeDailyError;

    fn write_chunk<'writer>(
        &'writer self,
        entity: DailySpendEntity,
        rows: &'writer [SpendRow],
        phase: &'writer CommitPhase,
    ) -> ChunkFuture<'writer, FakeDailyError> {
        let step = self.next_step();
        self.events
            .push(format!("daily {entity:?} {} {step:?}", rows.len()));
        Box::pin(async move { self.run_step(step, phase).await })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake window commit failed")]
pub(crate) struct FakeWindowError {
    pub(crate) failure: SpendFailure,
}

pub(crate) struct FakeWindow {
    events: Arc<Events>,
    failure: Mutex<Option<SpendFailure>>,
}

impl FakeWindow {
    pub(crate) fn new(events: Arc<Events>) -> Self {
        Self {
            events,
            failure: Mutex::new(None),
        }
    }

    pub(crate) fn fail_with(&self, failure: SpendFailure) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
    }
}

impl WindowSpendCommit for FakeWindow {
    type Error = FakeWindowError;

    fn commit_window_spend<'run>(
        &'run self,
        transactions: &'run [WindowSpendTransaction],
    ) -> WindowCommitFuture<'run, FakeWindowError> {
        self.events.push(format!("window {}", transactions.len()));
        let failure = *self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        Box::pin(ready(match failure {
            Some(kind) => Err(FakeWindowError { failure: kind }),
            None => Ok(()),
        }))
    }

    fn failure(&self, error: &FakeWindowError) -> SpendFailure {
        error.failure
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake tool discovery upsert failed")]
pub(crate) struct FakeDiscoveryError;

pub(crate) struct FakeDiscovery {
    events: Arc<Events>,
    failing: AtomicBool,
    tools: Mutex<Vec<String>>,
}

impl FakeDiscovery {
    pub(crate) fn new(events: Arc<Events>) -> Self {
        Self {
            events,
            failing: AtomicBool::new(false),
            tools: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn fail(&self) {
        self.failing.store(true, Ordering::SeqCst);
    }

    pub(crate) fn tools(&self) -> Vec<String> {
        self.tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ToolDiscoveryWriter for FakeDiscovery {
    type Error = FakeDiscoveryError;

    fn upsert_tools<'run>(
        &'run self,
        items: &'run [ToolDiscoveryItem],
    ) -> DiscoveryFuture<'run, FakeDiscoveryError> {
        self.events.push(format!("discovery {}", items.len()));
        self.tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(
                items
                    .iter()
                    .map(|item| item.tool_name.clone().unwrap_or_default()),
            );
        let failing = self.failing.load(Ordering::SeqCst);
        Box::pin(ready(if failing {
            Err(FakeDiscoveryError)
        } else {
            Ok(())
        }))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake spend logs service failed")]
pub(crate) struct FakeServiceError;

// What the fake's update pass does: settle at once, fail, or stand still until the test lets it go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateBehavior {
    Succeed,
    Block,
    Fail,
}

pub(crate) struct FakeService {
    events: Arc<Events>,
    census: Mutex<CensusPlan>,
    times: Mutex<Vec<TokioInstant>>,
    update: Mutex<UpdateBehavior>,
    drain_fails: AtomicBool,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

struct CensusPlan {
    scripted: VecDeque<i64>,
    last: i64,
}

impl FakeService {
    pub(crate) fn new(events: Arc<Events>, census: Vec<i64>) -> Self {
        Self {
            events,
            census: Mutex::new(CensusPlan {
                scripted: VecDeque::from(census),
                last: 0,
            }),
            times: Mutex::new(Vec::new()),
            update: Mutex::new(UpdateBehavior::Succeed),
            drain_fails: AtomicBool::new(false),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn block_updates(&self) {
        *self.update.lock().unwrap_or_else(PoisonError::into_inner) = UpdateBehavior::Block;
    }

    pub(crate) fn fail_updates(&self) {
        *self.update.lock().unwrap_or_else(PoisonError::into_inner) = UpdateBehavior::Fail;
    }

    pub(crate) fn fail_drains(&self) {
        self.drain_fails.store(true, Ordering::SeqCst);
    }

    // Wake the test once an update pass stands still, and let it go on the test's signal.
    pub(crate) fn entered(&self) -> Arc<Notify> {
        Arc::clone(&self.entered)
    }

    pub(crate) fn release(&self) -> Arc<Notify> {
        Arc::clone(&self.release)
    }

    // The paused clock reads, so a test compares the gaps between passes.
    pub(crate) fn census_times(&self) -> Vec<TokioInstant> {
        self.times
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next_census(&self) -> i64 {
        let mut plan = self.census.lock().unwrap_or_else(PoisonError::into_inner);
        let value = plan.scripted.pop_front().unwrap_or(plan.last);
        plan.last = value;
        value
    }

    async fn block_until_released(&self) -> Result<(), FakeServiceError> {
        self.entered.notify_one();
        self.release.notified().await;
        self.events.push("update finished".to_owned());
        Ok(())
    }
}

impl SpendLogsService for FakeService {
    type Error = FakeServiceError;

    fn recover_parked_logs(&self) -> ServiceFuture<'_, usize> {
        self.events.push("recover".to_owned());
        Box::pin(ready(0))
    }

    fn queued_transactions(&self) -> i64 {
        let value = self.next_census();
        self.times
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TokioInstant::now());
        self.events.push(format!("census {value}"));
        value
    }

    fn update_spend_logs(&self) -> ServiceFuture<'_, Result<(), FakeServiceError>> {
        let behavior = *self.update.lock().unwrap_or_else(PoisonError::into_inner);
        self.events.push(format!("update {behavior:?}"));
        match behavior {
            UpdateBehavior::Succeed => Box::pin(ready(Ok(()))),
            UpdateBehavior::Fail => Box::pin(ready(Err(FakeServiceError))),
            UpdateBehavior::Block => Box::pin(self.block_until_released()),
        }
    }

    fn run_spend_logs(&self) -> ServiceFuture<'_, Result<(), FakeServiceError>> {
        let failing = self.drain_fails.load(Ordering::SeqCst);
        self.events.push("drain".to_owned());
        Box::pin(ready(if failing {
            Err(FakeServiceError)
        } else {
            Ok(())
        }))
    }
}

// A Redis list port with no server: the appends are recorded and the pops find nothing.
#[derive(Default)]
pub(crate) struct RecordingLists {
    pushes: Mutex<Vec<String>>,
}

impl RecordingLists {
    pub(crate) fn pushes(&self) -> Vec<String> {
        self.pushes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendBufferLists for RecordingLists {
    fn pop<'call>(
        &'call self,
        _key: &'call str,
        _count: Option<i64>,
    ) -> SpendListFuture<'call, BufferReply> {
        Box::pin(ready(Ok(BufferReply::Missing)))
    }

    fn pop_pipeline<'call>(
        &'call self,
        operations: &'call [BufferPop],
    ) -> SpendListFuture<'call, Vec<BufferReply>> {
        let replies = operations.iter().map(|_pop| BufferReply::Missing).collect();
        Box::pin(ready(Ok(replies)))
    }

    fn push_and_trim<'call>(
        &'call self,
        key: &'call str,
        values: &'call [String],
        _max_len: i64,
    ) -> SpendListFuture<'call, i64> {
        let queued = {
            let mut pushes = self.pushes.lock().unwrap_or_else(PoisonError::into_inner);
            pushes.push(format!("{key} {}", values.len()));
            pushes.len()
        };
        let reply: Result<i64, SpendBufferListError> =
            Ok(i64::try_from(queued).unwrap_or(i64::MAX));
        Box::pin(ready(reply))
    }
}
