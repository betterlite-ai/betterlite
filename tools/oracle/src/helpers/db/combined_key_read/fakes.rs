use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use betterlite_db::auth_reconnect::request::ReconnectRequest;
use betterlite_db::auth_reconnect::stale::StaleReadEngine;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::combined_key_read::error::ReconnectFailure;
use betterlite_db::combined_key_read::grace::DeprecatedToken;
use betterlite_db::combined_key_read::ports::{
    DeprecatedFuture, Diagnostic, FailureHook, HookFuture, KeyFailureHook, KeyReadDatabase,
    KeyReadDiagnostics, PlanReconnect, ReconnectFuture, RowFuture,
};
use betterlite_db::database_failures::metrics::{
    DbMetricFuture, DbServiceEvent, DbServiceEventSink, DbServiceFailureEvent,
};
use betterlite_db::database_faults::DbError;
use betterlite_db::database_refresh::generation::Observation;
use betterlite_db::key_hash::KeyHash;
use betterlite_db::key_user_writes::retry::WriteClock;
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::tool_spend::RetryPause;
use chrono::{DateTime, Utc};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn bump(counter: &Mutex<u32>) {
    let mut value = lock(counter);
    *value = value.saturating_add(1);
}

#[derive(Debug, thiserror::Error)]
#[error("the helper's metric sink failed")]
pub(crate) struct SinkFailure;

#[derive(Default)]
pub(crate) struct RecordingSink {
    successes: Mutex<u32>,
    failures: Mutex<u32>,
}

impl RecordingSink {
    pub(crate) fn successes(&self) -> u32 {
        *lock(&self.successes)
    }

    pub(crate) fn failures(&self) -> u32 {
        *lock(&self.failures)
    }
}

impl DbServiceEventSink for RecordingSink {
    type Failure = SinkFailure;

    fn success(&self, _event: DbServiceEvent) -> DbMetricFuture<'_, Self::Failure> {
        bump(&self.successes);
        Box::pin(async { Ok(()) })
    }

    fn failure(&self, _event: DbServiceFailureEvent) -> DbMetricFuture<'_, Self::Failure> {
        bump(&self.failures);
        Box::pin(async { Ok(()) })
    }
}

pub(crate) struct StepClock {
    base: Instant,
    step: Duration,
    calls: Mutex<u32>,
    wall: DateTime<Utc>,
}

impl StepClock {
    pub(crate) fn new(step: Duration, wall: DateTime<Utc>) -> Self {
        Self {
            base: Instant::now(),
            step,
            calls: Mutex::new(0),
            wall,
        }
    }
}

impl WriteClock for StepClock {
    fn now(&self) -> DateTime<Utc> {
        self.wall
    }

    fn instant(&self) -> Instant {
        let mut calls = lock(&self.calls);
        let index = *calls;
        *calls = calls.saturating_add(1);
        drop(calls);
        self.base
            .checked_add(self.step.saturating_mul(index))
            .unwrap_or(self.base)
    }
}

pub(crate) struct FullJitter;

impl JitterSource for FullJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        upper_exclusive_nanos
    }
}

#[derive(Default)]
pub(crate) struct CountingPause {
    delays: Mutex<Vec<Duration>>,
}

impl CountingPause {
    pub(crate) fn pauses(&self) -> usize {
        lock(&self.delays).len()
    }
}

impl RetryPause for CountingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        lock(&self.delays).push(delay);
        Box::pin(async {})
    }
}

#[derive(Default)]
pub(crate) struct CountingDiagnostics {
    backoffs: Mutex<u32>,
    cached_plan: Mutex<u32>,
    deprecated_used: Mutex<bool>,
}

impl CountingDiagnostics {
    pub(crate) fn backoffs(&self) -> u32 {
        *lock(&self.backoffs)
    }

    pub(crate) fn cached_plan_warnings(&self) -> u32 {
        *lock(&self.cached_plan)
    }

    pub(crate) fn deprecated_used(&self) -> bool {
        *lock(&self.deprecated_used)
    }
}

impl KeyReadDiagnostics for CountingDiagnostics {
    fn emit(&self, diagnostic: Diagnostic<'_>) {
        match diagnostic {
            Diagnostic::Backoff { .. } => bump(&self.backoffs),
            Diagnostic::CachedPlan { .. } => bump(&self.cached_plan),
            Diagnostic::DeprecatedUsed => *lock(&self.deprecated_used) = true,
            Diagnostic::TokenHash
            | Diagnostic::AttemptFailure { .. }
            | Diagnostic::FailureTraceback { .. }
            | Diagnostic::DeprecatedLookupFailed { .. } => {}
        }
    }
}

#[derive(Default)]
pub(crate) struct CountingHook {
    hooks: Mutex<u32>,
}

impl CountingHook {
    pub(crate) fn hooks(&self) -> u32 {
        *lock(&self.hooks)
    }
}

impl KeyFailureHook for CountingHook {
    fn schedule(&self, _hook: FailureHook) -> HookFuture {
        bump(&self.hooks);
        Box::pin(async {})
    }
}

pub(crate) type RowStep = Result<Option<JsonData>, DbError>;
pub(crate) type DeprecatedStep = Result<Option<DeprecatedToken>, DbError>;

pub(crate) struct ScriptedDatabase {
    rows: Mutex<VecDeque<RowStep>>,
    deprecated: Mutex<VecDeque<DeprecatedStep>>,
    row_queries: Mutex<u32>,
    deprecated_queries: Mutex<u32>,
}

impl ScriptedDatabase {
    pub(crate) fn new(rows: Vec<RowStep>, deprecated: Vec<DeprecatedStep>) -> Self {
        Self {
            rows: Mutex::new(rows.into_iter().collect()),
            deprecated: Mutex::new(deprecated.into_iter().collect()),
            row_queries: Mutex::new(0),
            deprecated_queries: Mutex::new(0),
        }
    }

    pub(crate) fn row_queries(&self) -> u32 {
        *lock(&self.row_queries)
    }

    pub(crate) fn deprecated_queries(&self) -> u32 {
        *lock(&self.deprecated_queries)
    }
}

impl KeyReadDatabase for ScriptedDatabase {
    fn read_row<'lookup>(
        &'lookup self,
        _hash: &'lookup KeyHash,
        _use_writer: bool,
    ) -> RowFuture<'lookup> {
        bump(&self.row_queries);
        let next = lock(&self.rows).pop_front().unwrap_or(Ok(None));
        Box::pin(async move { next })
    }

    fn deprecated_row<'lookup>(
        &'lookup self,
        _hash: &'lookup KeyHash,
        _now: DateTime<Utc>,
        _use_writer: bool,
    ) -> DeprecatedFuture<'lookup> {
        bump(&self.deprecated_queries);
        let next = lock(&self.deprecated).pop_front().unwrap_or(Ok(None));
        Box::pin(async move { next })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedReconnect {
    pub(crate) reason: String,
    pub(crate) force_recreate: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) lock_timeout: Option<Duration>,
    pub(crate) stale_present: bool,
    pub(crate) stale_live: bool,
}

pub(crate) struct ScriptedReconnect {
    generation: u64,
    handle: Arc<u64>,
    results: Mutex<VecDeque<Result<bool, ReconnectFailure>>>,
    requests: Mutex<Vec<RecordedReconnect>>,
}

impl ScriptedReconnect {
    pub(crate) fn new(results: Vec<Result<bool, ReconnectFailure>>) -> Self {
        Self {
            generation: 1,
            handle: Arc::new(7),
            results: Mutex::new(results.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<RecordedReconnect> {
        lock(&self.requests).clone()
    }

    fn observation(&self) -> Observation<u64> {
        Observation {
            generation: self.generation,
            handle: Some(Arc::clone(&self.handle)),
            connected: true,
        }
    }
}

impl PlanReconnect for ScriptedReconnect {
    type Handle = u64;

    fn read_target(&self) -> Observation<u64> {
        self.observation()
    }

    fn attempt<'reconnect>(
        &'reconnect self,
        request: &'reconnect ReconnectRequest<'_>,
        stale: Option<&'reconnect StaleReadEngine<Self::Handle>>,
    ) -> ReconnectFuture<'reconnect> {
        let observed = self.observation();
        lock(&self.requests).push(RecordedReconnect {
            reason: request.reason.to_owned(),
            force_recreate: request.force_recreate,
            timeout: request.timeout,
            lock_timeout: request.lock_timeout,
            stale_present: stale.is_some(),
            stale_live: stale.is_some_and(|engine| engine.is_still_live(&observed)),
        });
        let next = lock(&self.results).pop_front().unwrap_or(Ok(true));
        Box::pin(async move { next })
    }
}
