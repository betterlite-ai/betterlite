use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use tracing::subscriber::DefaultGuard;

use crate::auth_reconnect::clock::ReconnectClock;
use crate::database_views::contracts::{
    ExecuteFuture, QueryFuture, RawRow, SupportsExecuteRaw, SupportsRawQueries,
};
use crate::db_lifecycle::health::{HealthCheck, HealthChecker, HealthFailure, HealthTarget};
use crate::db_lifecycle::read_only::{ReadOnlyWriter, ReconnectControl, RecreateState};
use crate::db_lifecycle::views::{
    VIEW_SETUP_DEADLINE, VIEW_SETUP_POLL_INTERVAL, ViewCatalog, ViewSetup,
};
use crate::db_lifecycle::watchdog::{Watchdog, WatchdogSettings};
use crate::db_lifecycle::{
    DatabaseFailure, HealthFailureReporter, LifecycleDatabase, LifecycleFuture, ProxyStateSink,
    TrackedTask,
};

#[derive(Debug, Clone, thiserror::Error)]
pub(super) enum FakeFailure {
    #[error("the connection dropped")]
    Dropped,
    #[error("the database is unavailable")]
    Unavailable,
    #[error("the caller sent a bad key")]
    Caller,
    #[error("{message}")]
    Message { message: String },
}

impl FakeFailure {
    pub(super) fn message(message: &str) -> Self {
        Self::Message {
            message: message.to_owned(),
        }
    }
}

impl DatabaseFailure for FakeFailure {
    fn is_connection_failure(&self) -> bool {
        matches!(self, Self::Dropped)
    }

    fn is_infrastructure_failure(&self) -> bool {
        matches!(self, Self::Dropped | Self::Unavailable)
    }
}

pub(super) enum Reply<Output> {
    Value(Output),
    After(Duration, Output),
    Pends,
}

async fn resolve<Output>(reply: Reply<Output>) -> Output {
    match reply {
        Reply::Value(output) => output,
        Reply::After(delay, output) => {
            sleep(delay).await;
            output
        }
        Reply::Pends => std::future::pending().await,
    }
}

#[derive(Default)]
pub(super) struct FakeState {
    pub(super) probe_replies: Mutex<VecDeque<Reply<Result<(), FakeFailure>>>>,
    pub(super) probe_calls: AtomicU32,
    pub(super) generation: AtomicU64,
    pub(super) generation_during_probe: Mutex<Option<u64>>,
    pub(super) replacement_settles: Mutex<Vec<Duration>>,
    pub(super) writer_read_only_replies: Mutex<VecDeque<Reply<Result<bool, FakeFailure>>>>,
    pub(super) writer_read_only_calls: AtomicU32,
    pub(super) writer_unavailable: AtomicBool,
    pub(super) row_count_replies: Mutex<VecDeque<Reply<Result<Option<i64>, FakeFailure>>>>,
    pub(super) row_count_calls: AtomicU32,
}

impl FakeState {
    pub(super) fn script_probe(&self, reply: Reply<Result<(), FakeFailure>>) {
        self.probe_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn script_failing_probes(&self, count: usize, failure: &FakeFailure) {
        for _ in 0..count {
            self.script_probe(Reply::Value(Err(failure.clone())));
        }
    }

    pub(super) fn script_writer_probe(&self, reply: Reply<Result<bool, FakeFailure>>) {
        self.writer_read_only_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn script_row_count(&self, reply: Reply<Result<Option<i64>, FakeFailure>>) {
        self.row_count_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn script_failing_row_counts(&self, count: usize, failure: &FakeFailure) {
        for _ in 0..count {
            self.script_row_count(Reply::Value(Err(failure.clone())));
        }
    }

    pub(super) fn script_missing_estimates(&self, count: usize) {
        for _ in 0..count {
            self.script_row_count(Reply::Value(Ok(None)));
        }
    }
}

#[derive(Clone)]
pub(super) struct FakeDatabase {
    pub(super) state: Arc<FakeState>,
}

impl LifecycleDatabase for FakeDatabase {
    type Error = FakeFailure;
    type Target = FakeTarget;

    fn probe_target(&self) -> FakeTarget {
        FakeTarget {
            state: Arc::clone(&self.state),
        }
    }

    fn writer_is_read_only(&self) -> LifecycleFuture<'_, Result<bool, FakeFailure>> {
        self.state
            .writer_read_only_calls
            .fetch_add(1, Ordering::SeqCst);
        let reply = self
            .state
            .writer_read_only_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::Value(Ok(false)));
        Box::pin(resolve(reply))
    }

    fn writer_unavailable(&self) -> bool {
        self.state.writer_unavailable.load(Ordering::SeqCst)
    }

    fn spend_logs_row_count(&self) -> LifecycleFuture<'_, Result<Option<i64>, FakeFailure>> {
        self.state.row_count_calls.fetch_add(1, Ordering::SeqCst);
        let reply = self
            .state
            .row_count_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::Value(Ok(None)));
        Box::pin(resolve(reply))
    }
}

pub(super) struct FakeTarget {
    state: Arc<FakeState>,
}

impl HealthTarget for FakeTarget {
    type Error = FakeFailure;

    fn generation(&self) -> u64 {
        self.state.generation.load(Ordering::SeqCst)
    }

    fn probe(&self) -> LifecycleFuture<'_, Result<(), FakeFailure>> {
        self.state.probe_calls.fetch_add(1, Ordering::SeqCst);
        let reply = self
            .state
            .probe_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::Value(Ok(())));
        let moved_generation = *self
            .state
            .generation_during_probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Box::pin(async move {
            if let Some(generation) = moved_generation {
                self.state.generation.store(generation, Ordering::SeqCst);
            }
            resolve(reply).await
        })
    }

    fn wait_for_replacement(&self, settle: Duration) -> LifecycleFuture<'_, ()> {
        self.state
            .replacement_settles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(settle);
        Box::pin(async move {})
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReconnectCall {
    pub(super) force_recreate: bool,
    pub(super) reason: String,
    pub(super) timeout: Option<Duration>,
}

pub(super) fn reconnect_call(reason: &str, timeout: Option<Duration>) -> ReconnectCall {
    ReconnectCall {
        force_recreate: false,
        reason: reason.to_owned(),
        timeout,
    }
}

pub(super) fn recreate_call(reason: &str, timeout: Option<Duration>) -> ReconnectCall {
    ReconnectCall {
        force_recreate: true,
        reason: reason.to_owned(),
        timeout,
    }
}

#[derive(Default)]
pub(super) struct FakeReconnect {
    pub(super) calls: Mutex<Vec<ReconnectCall>>,
    pub(super) results: Mutex<VecDeque<bool>>,
}

impl FakeReconnect {
    pub(super) fn called(&self) -> Vec<ReconnectCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn script_result(&self, result: bool) {
        self.results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(result);
    }

    fn record(&self, force_recreate: bool, reason: &str, timeout: Option<Duration>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ReconnectCall {
                force_recreate,
                reason: reason.to_owned(),
                timeout,
            });
    }

    fn next_result(&self) -> bool {
        self.results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(true)
    }

    fn call(&self, force_recreate: bool, reason: &str, timeout: Option<Duration>) -> bool {
        self.record(force_recreate, reason, timeout);
        self.next_result()
    }
}

impl ReconnectControl for FakeReconnect {
    fn reconnect<'borrow>(
        &'borrow self,
        reason: &'borrow str,
        timeout: Option<Duration>,
    ) -> LifecycleFuture<'borrow, bool> {
        Box::pin(async move { self.call(false, reason, timeout) })
    }

    fn force_recreate<'borrow>(
        &'borrow self,
        reason: &'borrow str,
        timeout: Option<Duration>,
    ) -> LifecycleFuture<'borrow, bool> {
        Box::pin(async move { self.call(true, reason, timeout) })
    }
}

#[derive(Clone)]
pub(super) struct FakeClock {
    pub(super) now: Arc<Mutex<Instant>>,
}

pub(super) fn fake_clock() -> FakeClock {
    FakeClock {
        now: Arc::new(Mutex::new(Instant::now())),
    }
}

impl FakeClock {
    pub(super) fn advance(&self, elapsed: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        let advanced = now
            .checked_add(elapsed)
            .expect("the fake clock never overflows");
        *now = advanced;
    }
}

impl ReconnectClock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
pub(super) struct FakeReporter {
    pub(super) failures: Mutex<Vec<HealthFailure>>,
    pub(super) started: AtomicU32,
    pub(super) gate: Mutex<Option<Arc<Notify>>>,
}

impl FakeReporter {
    pub(super) fn failures(&self) -> Vec<HealthFailure> {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl HealthFailureReporter for FakeReporter {
    fn report(&self, failure: HealthFailure) -> LifecycleFuture<'_, ()> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let gate = self
            .gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Box::pin(async move {
            if let Some(notify) = gate {
                notify.notified().await;
            }
            self.failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(failure);
        })
    }
}

#[derive(Default)]
pub(super) struct FakeSink {
    pub(super) row_counts: Mutex<Vec<i64>>,
}

impl ProxyStateSink for FakeSink {
    fn set_spend_logs_row_count(&self, row_count: i64) {
        self.row_counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(row_count);
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeHealth {
    pub(super) actions: Arc<Mutex<Vec<String>>>,
    pub(super) outcome: Arc<Mutex<Option<FakeFailure>>>,
    pub(super) row_count: Arc<AtomicI64>,
}

impl HealthCheck for FakeHealth {
    type Error = FakeFailure;

    fn health_check(&self) -> LifecycleFuture<'_, Result<(), FakeFailure>> {
        Box::pin(async move {
            self.actions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push("health_check".to_owned());
            self.outcome
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .map_or(Ok(()), Err)
        })
    }

    fn publish_spend_logs_row_count<'borrow>(
        &'borrow self,
        sink: &'borrow dyn ProxyStateSink,
    ) -> LifecycleFuture<'borrow, ()> {
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("publish".to_owned());
        sink.set_spend_logs_row_count(self.row_count.load(Ordering::SeqCst));
        Box::pin(async move {})
    }
}

#[derive(Default)]
pub(super) struct FakeCatalogState {
    pub(super) actions: Arc<Mutex<Vec<String>>>,
    pub(super) gate_replies: Mutex<VecDeque<Reply<Result<bool, FakeFailure>>>>,
    pub(super) known_views: Mutex<Vec<String>>,
    pub(super) existing_failure: Mutex<Option<FakeFailure>>,
    pub(super) schemas: Mutex<Vec<String>>,
    pub(super) queries: Mutex<VecDeque<Result<Vec<RawRow>, FakeFailure>>>,
    pub(super) executed: Mutex<Vec<String>>,
    pub(super) creates: Mutex<VecDeque<Result<(), FakeFailure>>>,
}

impl FakeCatalogState {
    pub(super) fn script_gate(&self, reply: Reply<Result<bool, FakeFailure>>) {
        self.gate_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn script_estimate(&self, reltuples: i64) {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(vec![RawRow {
                columns: BTreeMap::from([("reltuples".to_owned(), Some(reltuples))]),
            }]));
    }

    pub(super) fn script_missing_view(&self) {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(FakeFailure::message(
                "relation \"view\" does not exist",
            )));
    }

    pub(super) fn script_missing_views(&self, count: usize) {
        for _ in 0..count {
            self.script_missing_view();
        }
    }

    pub(super) fn script_successful_creates(&self, count: usize) {
        for _ in 0..count {
            self.script_create(Ok(()));
        }
    }

    pub(super) fn script_missing_gate(&self, count: usize) {
        for _ in 0..count {
            self.script_gate(Reply::Value(Ok(false)));
        }
    }

    pub(super) fn script_create(&self, reply: Result<(), FakeFailure>) {
        self.creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn created(&self) -> Vec<String> {
        self.executed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn take_creation(&self) -> Result<(), FakeFailure> {
        self.creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn take_query(&self) -> Result<Vec<RawRow>, FakeFailure> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(FakeFailure::message("no query was scripted")))
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeCatalog {
    pub(super) state: Arc<FakeCatalogState>,
}

impl ViewCatalog for FakeCatalog {
    fn gate_table_present(&self) -> LifecycleFuture<'_, Result<bool, FakeFailure>> {
        self.state
            .actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("gate".to_owned());
        let reply = self
            .state
            .gate_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::Value(Ok(true)));
        Box::pin(resolve(reply))
    }

    fn existing_views<'borrow>(
        &'borrow self,
        schema: &'borrow str,
        expected: &'borrow [&'borrow str],
    ) -> LifecycleFuture<'borrow, Result<Vec<String>, FakeFailure>> {
        self.state
            .actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("views".to_owned());
        self.state
            .schemas
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(schema.to_owned());
        let known = self
            .state
            .known_views
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let names: Vec<String> = known
            .into_iter()
            .filter(|name| expected.iter().any(|view| *view == name.as_str()))
            .collect();
        let failure = self
            .state
            .existing_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let outcome = failure.map_or(Ok(names), Err);
        Box::pin(async move { outcome })
    }
}

impl SupportsExecuteRaw for FakeCatalog {
    type Error = FakeFailure;

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> ExecuteFuture<'database, FakeFailure> {
        self.state
            .executed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(query.to_owned());
        Box::pin(async move { self.state.take_creation() })
    }
}

impl SupportsRawQueries for FakeCatalog {
    fn query_raw<'database>(
        &'database self,
        _query: &'database str,
    ) -> QueryFuture<'database, FakeFailure> {
        Box::pin(async move { self.state.take_query() })
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(super) fn lines(&self) -> Vec<String> {
        let text = String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap();
        text.lines().map(str::to_owned).collect()
    }

    fn record(&self, buffer: &[u8]) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.record(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// the guard is thread-local, so a task spawned on the test's current-thread runtime records into it too
pub(super) fn capture_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_level(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (logs, guard)
}

pub(super) const WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const RECONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const COOLDOWN: Duration = Duration::from_secs(15);

pub(super) fn watchdog_settings() -> WatchdogSettings {
    WatchdogSettings {
        enabled: true,
        interval: WATCHDOG_INTERVAL,
        probe_timeout: PROBE_TIMEOUT,
        reconnect_timeout: RECONNECT_TIMEOUT,
    }
}

pub(super) struct WatchdogHarness {
    pub(super) state: Arc<FakeState>,
    pub(super) reconnect: Arc<FakeReconnect>,
    pub(super) read_only: Arc<ReadOnlyWriter<FakeReconnect>>,
    pub(super) watchdog: Arc<Watchdog<FakeDatabase, FakeReconnect>>,
    pub(super) tasks: TaskTracker,
}

pub(super) fn watchdog_harness() -> WatchdogHarness {
    watchdog_harness_with(watchdog_settings())
}

pub(super) fn watchdog_harness_with(settings: WatchdogSettings) -> WatchdogHarness {
    let state = Arc::new(FakeState::default());
    let reconnect = Arc::new(FakeReconnect::default());
    let read_only = Arc::new(ReadOnlyWriter {
        reconnect: Arc::clone(&reconnect),
        cooldown: COOLDOWN,
        clock: crate::auth_reconnect::clock::SystemClock,
        state: Mutex::new(RecreateState::default()),
    });
    let watchdog = Arc::new(Watchdog {
        database: FakeDatabase {
            state: Arc::clone(&state),
        },
        reconnect: Arc::clone(&reconnect),
        read_only: Arc::clone(&read_only),
        settings,
        task: TrackedTask {
            handle: Mutex::new(None),
        },
    });
    WatchdogHarness {
        state,
        reconnect,
        read_only,
        watchdog,
        tasks: TaskTracker::new(),
    }
}

pub(super) struct HealthHarness {
    pub(super) state: Arc<FakeState>,
    pub(super) reporter: Arc<FakeReporter>,
    pub(super) checker: HealthChecker<FakeDatabase>,
}

pub(super) fn health_harness() -> HealthHarness {
    let state = Arc::new(FakeState::default());
    let reporter = Arc::new(FakeReporter::default());
    let checker = HealthChecker {
        database: FakeDatabase {
            state: Arc::clone(&state),
        },
        reporter: Arc::<FakeReporter>::clone(&reporter),
        tasks: TaskTracker::new(),
    };
    HealthHarness {
        state,
        reporter,
        checker,
    }
}

pub(super) struct ViewsHarness {
    pub(super) actions: Arc<Mutex<Vec<String>>>,
    pub(super) catalog: FakeCatalog,
    pub(super) health: FakeHealth,
    pub(super) sink: Arc<FakeSink>,
    pub(super) setup: Arc<ViewSetup<FakeCatalog, FakeHealth>>,
}

pub(super) fn views_harness() -> ViewsHarness {
    views_harness_with_deadline(VIEW_SETUP_DEADLINE)
}

pub(super) fn views_harness_with_deadline(deadline: Duration) -> ViewsHarness {
    let catalog = FakeCatalog::default();
    let actions = Arc::clone(&catalog.state.actions);
    let health = FakeHealth {
        actions: Arc::clone(&actions),
        outcome: Arc::new(Mutex::new(None)),
        row_count: Arc::new(AtomicI64::new(7)),
    };
    let sink = Arc::new(FakeSink::default());
    let setup = Arc::new(ViewSetup {
        database: catalog.clone(),
        health: health.clone(),
        sink: Arc::<FakeSink>::clone(&sink),
        schema: "public".to_owned(),
        poll_interval: VIEW_SETUP_POLL_INTERVAL,
        deadline,
        task: TrackedTask {
            handle: Mutex::new(None),
        },
    });
    ViewsHarness {
        actions,
        catalog,
        health,
        sink,
        setup,
    }
}
