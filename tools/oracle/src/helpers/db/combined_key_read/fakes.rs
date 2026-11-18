use super::trace::{Trace, record};
use betterlite_db::auth_reconnect::request::ReconnectRequest;
use betterlite_db::auth_reconnect::stale::StaleReadEngine;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::combined_key_read::error::{DatabaseFailure, ReconnectFailure};
use betterlite_db::combined_key_read::grace::DeprecatedToken;
use betterlite_db::combined_key_read::ports::{
    DeprecatedFuture, Diagnostic, FailureHook, HookFuture, KeyFailureHook, KeyReadDatabase,
    KeyReadDiagnostics, PlanReconnect, ReconnectFuture, RowFuture,
};
use betterlite_db::combined_key_read::postgres::DEPRECATED_TOKEN_TABLE;
use betterlite_db::combined_key_read::request::COMBINED_VIEW_TABLE;
use betterlite_db::database_failures::metrics::{
    DbMetricFuture, DbServiceEvent, DbServiceEventSink, DbServiceFailureEvent,
};
use betterlite_db::database_refresh::generation::Observation;
use betterlite_db::key_hash::KeyHash;
use betterlite_db::key_user_writes::retry::WriteClock;
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::tool_spend::RetryPause;
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use serde::Serialize;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
#[derive(Debug, thiserror::Error)]
#[error("the helper's metric sink failed")]
pub(crate) struct SinkFailure;
#[derive(Default)]
pub(crate) struct RecordingSink;
impl DbServiceEventSink for RecordingSink {
    type Failure = SinkFailure;
    fn success(&self, _event: DbServiceEvent) -> DbMetricFuture<'_, Self::Failure> {
        Box::pin(async { Ok(()) })
    }
    fn failure(&self, _event: DbServiceFailureEvent) -> DbMetricFuture<'_, Self::Failure> {
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

// the pause yields so the hook task the reader detached runs before the next attempt, as the source's does
#[derive(Default)]
pub(crate) struct NoopPause;

impl RetryPause for NoopPause {
    fn pause(&self, _delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async { tokio::task::yield_now().await })
    }
}

#[derive(Clone)]
pub(crate) struct RecordingDiagnostics {
    trace: Trace,
}

impl RecordingDiagnostics {
    pub(crate) fn new(trace: &Trace) -> Self {
        Self {
            trace: Arc::clone(trace),
        }
    }
}

impl KeyReadDiagnostics for RecordingDiagnostics {
    fn emit(&self, diagnostic: Diagnostic<'_>) {
        let label = match diagnostic {
            Diagnostic::TokenHash => "token_hash",
            Diagnostic::CachedPlan { .. } => "cached_plan",
            Diagnostic::AttemptFailure { .. } | Diagnostic::FailureTraceback { .. } => "failure",
            Diagnostic::Backoff { .. } => "backoff",
            Diagnostic::DeprecatedUsed => "deprecated_used",
            Diagnostic::DeprecatedLookupFailed { .. } => "deprecated_failed",
        };
        record(&self.trace, label);
    }
}

// the event is recorded on the first poll, when the detached hook would start running on the source
#[derive(Clone)]
pub(crate) struct RecordingHook {
    trace: Trace,
}

impl RecordingHook {
    pub(crate) fn new(trace: &Trace) -> Self {
        Self {
            trace: Arc::clone(trace),
        }
    }
}

impl KeyFailureHook for RecordingHook {
    fn schedule(&self, hook: FailureHook) -> HookFuture {
        let trace = Arc::clone(&self.trace);
        let event = format!("hook:{}", hook.call_type);
        Box::pin(async move { record(&trace, &event) })
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct RecordedQuery {
    pub(crate) target: &'static str,
    pub(crate) token_hash: String,
    pub(crate) use_writer: bool,
    pub(crate) now_seconds: Option<i64>,
}

pub(crate) type RowStep = Result<Option<JsonData>, DatabaseFailure>;
pub(crate) type DeprecatedStep = Result<Option<DeprecatedToken>, DatabaseFailure>;

pub(crate) struct ScriptedDatabase {
    rows: Mutex<VecDeque<RowStep>>,
    deprecated: Mutex<VecDeque<DeprecatedStep>>,
    queries: Mutex<Vec<RecordedQuery>>,
}

impl ScriptedDatabase {
    pub(crate) fn new(rows: Vec<RowStep>, deprecated: Vec<DeprecatedStep>) -> Self {
        Self {
            rows: Mutex::new(rows.into_iter().collect()),
            deprecated: Mutex::new(deprecated.into_iter().collect()),
            queries: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn queries(&self) -> Vec<RecordedQuery> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record_query(
        &self,
        target: &'static str,
        hash: &KeyHash,
        use_writer: bool,
        now_seconds: Option<i64>,
    ) {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedQuery {
                target,
                token_hash: hash.expose_secret().to_owned(),
                use_writer,
                now_seconds,
            });
    }
}

fn scripted_step<T>(steps: &Mutex<VecDeque<T>>, fallback: T) -> T {
    let mut queue = steps.lock().unwrap_or_else(PoisonError::into_inner);
    queue.pop_front().unwrap_or(fallback)
}

impl KeyReadDatabase for ScriptedDatabase {
    fn read_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        use_writer: bool,
    ) -> RowFuture<'lookup> {
        self.record_query(COMBINED_VIEW_TABLE, hash, use_writer, None);
        let next: RowStep = scripted_step(&self.rows, Ok(None));
        Box::pin(async move { next })
    }

    fn deprecated_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        now: DateTime<Utc>,
        use_writer: bool,
    ) -> DeprecatedFuture<'lookup> {
        self.record_query(
            DEPRECATED_TOKEN_TABLE,
            hash,
            use_writer,
            Some(now.timestamp()),
        );
        let next: DeprecatedStep = scripted_step(&self.deprecated, Ok(None));
        // the SQL keeps only rows whose revoke_at is still ahead of the lookup's now
        let next = match next {
            Ok(Some(row)) if row.revoke_at <= now => Ok(None),
            other => other,
        };
        Box::pin(async move { next })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedReconnect {
    pub(crate) reason: String,
    pub(crate) force: bool,
    pub(crate) force_recreate: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) lock_timeout: Option<Duration>,
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
            force: request.force,
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
        drop(results);
        Box::pin(async move { next })
    }
}
