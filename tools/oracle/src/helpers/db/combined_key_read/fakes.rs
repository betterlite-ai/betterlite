use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
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

#[derive(Debug, thiserror::Error)]
#[error("the helper's metric sink failed")]
pub(crate) struct SinkFailure;

#[derive(Default)]
pub(crate) struct RecordingSink {
    pub(crate) successes: AtomicU32,
    pub(crate) failures: AtomicU32,
}

impl DbServiceEventSink for RecordingSink {
    type Failure = SinkFailure;

    fn success(&self, _event: DbServiceEvent) -> DbMetricFuture<'_, Self::Failure> {
        self.successes.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Ok(()) })
    }

    fn failure(&self, _event: DbServiceFailureEvent) -> DbMetricFuture<'_, Self::Failure> {
        self.failures.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Ok(()) })
    }
}

pub(crate) struct StepClock {
    base: Instant,
    step: Duration,
    calls: AtomicU32,
    wall: DateTime<Utc>,
}

impl StepClock {
    pub(crate) fn new(step: Duration, wall: DateTime<Utc>) -> Self {
        Self {
            base: Instant::now(),
            step,
            calls: AtomicU32::new(0),
            wall,
        }
    }
}

impl WriteClock for StepClock {
    fn now(&self) -> DateTime<Utc> {
        self.wall
    }

    fn instant(&self) -> Instant {
        let index = self.calls.fetch_add(1, Ordering::Relaxed);
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
pub(crate) struct NoopPause;

impl RetryPause for NoopPause {
    fn pause(&self, _delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }
}

#[derive(Default)]
pub(crate) struct CountingDiagnostics {
    pub(crate) backoffs: AtomicU32,
    pub(crate) cached_plan: AtomicU32,
    pub(crate) deprecated_used: AtomicBool,
}

impl KeyReadDiagnostics for CountingDiagnostics {
    fn emit(&self, diagnostic: Diagnostic<'_>) {
        match diagnostic {
            Diagnostic::Backoff { .. } => {
                self.backoffs.fetch_add(1, Ordering::Relaxed);
            }
            Diagnostic::CachedPlan { .. } => {
                self.cached_plan.fetch_add(1, Ordering::Relaxed);
            }
            Diagnostic::DeprecatedUsed => {
                self.deprecated_used.store(true, Ordering::Relaxed);
            }
            Diagnostic::TokenHash
            | Diagnostic::AttemptFailure { .. }
            | Diagnostic::FailureTraceback { .. }
            | Diagnostic::DeprecatedLookupFailed { .. } => {}
        }
    }
}

#[derive(Default)]
pub(crate) struct CountingHook {
    pub(crate) hooks: AtomicU32,
}

impl KeyFailureHook for CountingHook {
    fn schedule(&self, _hook: FailureHook) -> HookFuture {
        self.hooks.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

pub(crate) type RowStep = Result<Option<JsonData>, DbError>;
pub(crate) type DeprecatedStep = Result<Option<DeprecatedToken>, DbError>;

pub(crate) struct ScriptedDatabase {
    pub(crate) rows: Mutex<VecDeque<RowStep>>,
    pub(crate) deprecated: Mutex<VecDeque<DeprecatedStep>>,
    pub(crate) row_queries: AtomicU32,
    pub(crate) deprecated_queries: AtomicU32,
}

impl ScriptedDatabase {
    pub(crate) fn new(rows: Vec<RowStep>, deprecated: Vec<DeprecatedStep>) -> Self {
        Self {
            rows: Mutex::new(rows.into_iter().collect()),
            deprecated: Mutex::new(deprecated.into_iter().collect()),
            row_queries: AtomicU32::new(0),
            deprecated_queries: AtomicU32::new(0),
        }
    }
}

fn scripted_step<T>(steps: &Mutex<VecDeque<T>>, fallback: T) -> T {
    let mut queue = steps.lock().unwrap_or_else(PoisonError::into_inner);
    let next = queue.pop_front();
    next.unwrap_or(fallback)
}

impl KeyReadDatabase for ScriptedDatabase {
    fn read_row<'lookup>(
        &'lookup self,
        _hash: &'lookup KeyHash,
        _use_writer: bool,
    ) -> RowFuture<'lookup> {
        self.row_queries.fetch_add(1, Ordering::Relaxed);
        let next: RowStep = scripted_step(&self.rows, Ok(None));
        Box::pin(async move { next })
    }

    fn deprecated_row<'lookup>(
        &'lookup self,
        _hash: &'lookup KeyHash,
        _now: DateTime<Utc>,
        _use_writer: bool,
    ) -> DeprecatedFuture<'lookup> {
        let next: DeprecatedStep = scripted_step(&self.deprecated, Ok(None));
        self.deprecated_queries.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move { next })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedReconnect {
    pub(crate) reason: String,
    pub(crate) force_recreate: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) lock_timeout: Option<Duration>,
    // Some(live) when the reconnect named a stale engine, None when it did not
    pub(crate) stale: Option<bool>,
}

pub(crate) struct ScriptedReconnect {
    generation: u64,
    handle: Arc<u64>,
    results: Mutex<VecDeque<Result<bool, ReconnectFailure>>>,
    pub(crate) requests: Mutex<Vec<RecordedReconnect>>,
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
        let recorded = RecordedReconnect {
            reason: request.reason.to_owned(),
            force_recreate: request.force_recreate,
            timeout: request.timeout,
            lock_timeout: request.lock_timeout,
            stale: stale.map(|engine| engine.is_still_live(&observed)),
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        let mut results = self.results.lock().unwrap_or_else(PoisonError::into_inner);
        let next = results.pop_front().unwrap_or(Ok(true));
        Box::pin(async move { next })
    }
}
