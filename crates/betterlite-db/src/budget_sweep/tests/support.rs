use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, pending};
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use tokio::sync::Notify;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;
use uuid::Uuid;

use crate::budget_reset_policy::cache_invalidation::{CacheDeleteFuture, ResetCache};
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_reset_policy::phase_failure::{PhaseFailureEvent, PhaseFailureSink};
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::budget_sweep::phase::{PhaseRunner, ResetPhase, SweepPorts};
use crate::budget_sweep::production::SweepInputs;
use crate::budget_sweep::settings::{BudgetResetSettings, ResetSettingsSource};
use crate::database_faults::DbError;
use crate::database_faults::reconnect::DbReconnect;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::service_hooks::NoPhaseSuccessSink;
use crate::postgres_store::session::PostgresStore;
use crate::queues::pod_lock::owner::PodId;
use crate::queues::pod_lock::{LockCache, LockCacheError, LockScript, PodLockManager};
use crate::team_window_reset::cache::{CounterReadFuture, CounterWriteFuture, WindowCounterCache};
use crate::team_window_reset::window_cache_roll::WindowDurationSeconds;

// A pool that never connects: construction is enough for tests that only carry the handle, and a phase that reads through it fails at once.
pub(crate) fn lazy_store() -> PostgresStore {
    let config = "postgres://betterlite:betterlite@127.0.0.1:9/betterlite"
        .parse()
        .unwrap();
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    PostgresStore { pool: Some(pool) }
}

pub(crate) fn moment() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap_or_default()
}

pub(crate) fn pod_id() -> PodId {
    PodId::try_from(Uuid::new_v4()).unwrap()
}

#[derive(Clone)]
pub(crate) struct Recorder<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for Recorder<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<T: Clone> Recorder<T> {
    fn push(&self, value: T) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
    }

    fn values(&self) -> Vec<T> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct FixedSchedule {
    pub(crate) next: DateTime<Utc>,
}

impl ResetSchedule for FixedSchedule {
    type Failure = Infallible;

    fn next_reset_at(&self, _budget_duration: &str) -> Result<DateTime<Utc>, Infallible> {
        Ok(self.next)
    }
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<Utc>,
}

impl RefreshClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

pub(crate) struct FixedDurations;

impl WindowDurationSeconds for FixedDurations {
    type Failure = Infallible;

    fn duration_seconds(&self, _budget_duration: &str) -> Result<i64, Infallible> {
        Ok(3600)
    }
}

pub(crate) struct NoReconnect;

impl DbReconnect for NoReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        _reason: &'client str,
        _timeout_seconds: f64,
        _lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        Box::pin(async { Ok(false) })
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingFailures(Recorder<PhaseFailureEvent>);

impl RecordingFailures {
    pub(crate) fn call_types(&self) -> Vec<String> {
        self.0
            .values()
            .into_iter()
            .map(|event| event.call_type)
            .collect()
    }
}

impl PhaseFailureSink for RecordingFailures {
    fn emit(&self, event: PhaseFailureEvent) {
        self.0.push(event);
    }
}

// The single cache port both reset phases share: invalidation succeeds silently and window counters live in memory.
#[derive(Default)]
pub(crate) struct SweepCache {
    counters: Mutex<BTreeMap<String, Decimal>>,
}

impl ResetCache for SweepCache {
    type Failure = Infallible;

    fn delete_spend_counters<'cache>(
        &'cache self,
        _keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        _keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl WindowCounterCache for SweepCache {
    type Failure = Infallible;

    fn set_memory(&self, key: &str, value: Decimal) {
        self.counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    fn set_redis<'cache>(
        &'cache self,
        key: &'cache str,
        value: Decimal,
    ) -> CounterWriteFuture<'cache, Infallible> {
        self.counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        Box::pin(async { Ok(()) })
    }

    fn read<'cache>(&'cache self, key: &'cache str) -> CounterReadFuture<'cache, Infallible> {
        let value = self
            .counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied();
        Box::pin(async move { Ok(value) })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LeaseOperation {
    Set(String, i64),
    Read(String),
    Delete(String),
}

pub(crate) struct RecordingLeaseCache {
    operations: Recorder<LeaseOperation>,
    granted: bool,
    read_fails: bool,
    owner: Mutex<Option<Vec<u8>>>,
}

impl RecordingLeaseCache {
    pub(crate) fn granting() -> Self {
        Self::held_by(None, true, false)
    }

    pub(crate) fn denied() -> Self {
        Self::held_by(None, false, false)
    }

    pub(crate) fn denied_with_owner(owner: &str) -> Self {
        Self::held_by(Some(owner.as_bytes().to_vec()), false, false)
    }

    pub(crate) fn denied_with_raw_owner(owner: Vec<u8>) -> Self {
        Self::held_by(Some(owner), false, false)
    }

    pub(crate) fn unreadable() -> Self {
        Self::held_by(None, false, true)
    }

    fn held_by(owner: Option<Vec<u8>>, granted: bool, read_fails: bool) -> Self {
        Self {
            operations: Recorder::default(),
            granted,
            read_fails,
            owner: Mutex::new(owner),
        }
    }

    fn record(&self, operation: LeaseOperation) {
        self.operations.push(operation);
    }

    pub(crate) fn operations(&self) -> Vec<LeaseOperation> {
        self.operations.values()
    }
}

impl LockCache for RecordingLeaseCache {
    async fn get_owner(&self, key: &str) -> Result<Option<Vec<u8>>, LockCacheError> {
        self.record(LeaseOperation::Read(key.to_owned()));
        if self.read_fails {
            return Err(LockCacheError::UnexpectedReply {
                operation: "get_owner",
            });
        }
        let guard = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(guard.clone())
    }

    async fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> Result<bool, LockCacheError> {
        self.record(LeaseOperation::Set(key.to_owned(), ttl_seconds));
        if !self.granted {
            return Ok(false);
        }
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(owner.as_bytes().to_vec());
        Ok(true)
    }

    async fn delete(&self, key: &str) -> Result<i64, LockCacheError> {
        self.record(LeaseOperation::Delete(key.to_owned()));
        let mut guard = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
        let deleted = guard.take().is_some();
        Ok(i64::from(deleted))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhaseEvent {
    Start(ResetPhase),
    End(ResetPhase),
}

impl PhaseEvent {
    const fn started(self) -> Option<ResetPhase> {
        match self {
            Self::Start(phase) => Some(phase),
            Self::End(_) => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct PhaseFault {
    pub(crate) message: String,
}

#[derive(Clone)]
struct FailureAt {
    phase: ResetPhase,
    message: String,
}

#[derive(Clone)]
struct Blocking {
    phase: ResetPhase,
    started: Arc<Notify>,
}

#[derive(Clone, Default)]
pub(crate) struct RecordingPhases {
    timeline: Recorder<PhaseEvent>,
    inside: Arc<AtomicBool>,
    overlapped: Arc<AtomicBool>,
    failure: Option<FailureAt>,
    blocking: Option<Blocking>,
}

impl RecordingPhases {
    fn new(failure: Option<FailureAt>, blocking: Option<Blocking>) -> Self {
        Self {
            timeline: Recorder::default(),
            inside: Arc::new(AtomicBool::new(false)),
            overlapped: Arc::new(AtomicBool::new(false)),
            failure,
            blocking,
        }
    }

    pub(crate) fn failing_at(phase: ResetPhase, message: &str) -> Self {
        Self::new(
            Some(FailureAt {
                phase,
                message: message.to_owned(),
            }),
            None,
        )
    }

    pub(crate) fn blocking_at(phase: ResetPhase, started: Arc<Notify>) -> Self {
        Self::new(None, Some(Blocking { phase, started }))
    }

    fn record(&self, event: PhaseEvent) {
        self.timeline.push(event);
    }

    pub(crate) fn phases(&self) -> Vec<ResetPhase> {
        self.timeline()
            .into_iter()
            .filter_map(PhaseEvent::started)
            .collect()
    }

    pub(crate) fn timeline(&self) -> Vec<PhaseEvent> {
        self.timeline.values()
    }

    pub(crate) fn overlapped(&self) -> bool {
        self.overlapped.load(Ordering::SeqCst)
    }
}

impl PhaseRunner for RecordingPhases {
    type Failure = PhaseFault;

    async fn run(&mut self, phase: ResetPhase) -> Result<(), PhaseFault> {
        if self.inside.swap(true, Ordering::SeqCst) {
            self.overlapped.store(true, Ordering::SeqCst);
        }
        self.record(PhaseEvent::Start(phase));
        if let Some(blocking) = self
            .blocking
            .as_ref()
            .filter(|blocking| blocking.phase == phase)
        {
            blocking.started.notify_one();
            pending::<()>().await;
        }
        let outcome = self
            .failure
            .as_ref()
            .filter(|failure| failure.phase == phase)
            .map_or(Ok(()), |failure| {
                Err(PhaseFault {
                    message: failure.message.clone(),
                })
            });
        self.record(PhaseEvent::End(phase));
        self.inside.store(false, Ordering::SeqCst);
        outcome
    }
}

pub(crate) struct TestSnapshot {
    settings: BudgetResetSettings,
    calls: AtomicUsize,
}

impl TestSnapshot {
    pub(crate) fn new(settings: BudgetResetSettings) -> Self {
        Self {
            settings,
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ResetSettingsSource for TestSnapshot {
    fn budget_reset_settings(&self) -> BudgetResetSettings {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.settings.clone()
    }
}

pub(crate) type AssemblyInputs = SweepInputs<
    RecordingLeaseCache,
    FixedSchedule,
    SweepCache,
    RecordingFailures,
    NoPhaseSuccessSink,
    NoReconnect,
    FixedClock,
    FixedDurations,
>;

pub(crate) fn assembly_inputs(
    database: PostgresStore,
    settings: Option<BudgetResetSettings>,
    lock_manager: Option<Arc<PodLockManager<RecordingLeaseCache>>>,
    tasks: TaskTracker,
) -> AssemblyInputs {
    SweepInputs {
        database,
        settings,
        lock_manager,
        ports: SweepPorts {
            limits: ResetLimits::default(),
            rollover: RolloverPolicy::default(),
            default_enduser_budget_id: None,
            schedule: FixedSchedule { next: moment() },
            cache: SweepCache::default(),
            failure: RecordingFailures::default(),
            success: NoPhaseSuccessSink,
            reconnect: None,
            clock: FixedClock { now: moment() },
            durations: FixedDurations,
        },
        tasks,
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn captured_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let guard = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(Level::TRACE)
        .set_default();
    (logs, guard)
}
