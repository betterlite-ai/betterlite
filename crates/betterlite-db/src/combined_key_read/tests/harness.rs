#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;

use crate::auth_reconnect::request::ReconnectRequest;
use crate::auth_reconnect::stale::StaleReadEngine;
use crate::budget_permission_rows::JsonData;
use crate::database_failures::metrics::{
    DbMetricFuture, DbServiceEvent, DbServiceEventSink, DbServiceFailureEvent,
};
use crate::database_faults::{DbError, ErrorType};
use crate::database_refresh::generation::Observation;
use crate::key_hash::KeyHash;
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::lock::JitterSource;
use crate::tool_spend::RetryPause;

use crate::combined_key_read::error::{DatabaseFailure, KeyReadError, ReconnectFailure};
use crate::combined_key_read::grace::{DeprecatedKeyCache, DeprecatedToken};
use crate::combined_key_read::ports::{
    DeprecatedFuture, Diagnostic, FailureHook, HookFuture, KeyFailureHook, KeyReadDatabase,
    KeyReadDiagnostics, NoSpanResolver, PlanReconnect, ReconnectFuture, RowFuture,
};
use crate::combined_key_read::reader::{KeyReader, ReadServices};
use crate::combined_key_read::request::CombinedKeyRequest;
use crate::combined_key_read::view::CombinedKeyView;

pub(crate) type Timeline = Arc<Mutex<Vec<String>>>;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn events(timeline: &Timeline) -> Vec<String> {
    lock(timeline).clone()
}

pub(crate) fn fixed_now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

pub(crate) struct StepClock {
    base: Instant,
    step: Duration,
    calls: Mutex<u32>,
    wall: Mutex<DateTime<Utc>>,
}

impl StepClock {
    pub(crate) fn new(step: Duration, wall: DateTime<Utc>) -> Self {
        Self {
            base: Instant::now(),
            step,
            calls: Mutex::new(0),
            wall: Mutex::new(wall),
        }
    }

    pub(crate) fn advance_wall(&self, by: Duration) {
        let mut wall = lock(&self.wall);
        *wall = wall
            .checked_add_signed(chrono::TimeDelta::from_std(by).unwrap_or_default())
            .unwrap_or(*wall);
    }
}

impl WriteClock for StepClock {
    fn now(&self) -> DateTime<Utc> {
        *lock(&self.wall)
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

pub(crate) struct RecordingPause {
    pub(crate) delays: Mutex<Vec<Duration>>,
    pub(crate) timeline: Timeline,
}

impl RecordingPause {
    pub(crate) fn shared(timeline: &Timeline) -> Self {
        Self {
            delays: Mutex::new(Vec::new()),
            timeline: Arc::clone(timeline),
        }
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        lock(&self.delays).push(delay);
        lock(&self.timeline).push(String::from("pause"));
        Box::pin(async {})
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ObservedDiagnostic {
    TokenHash,
    CachedPlan(String),
    AttemptFailure(String),
    FailureTraceback(String),
    Backoff(u32),
    DeprecatedUsed,
    DeprecatedLookupFailed(String),
}

impl From<Diagnostic<'_>> for ObservedDiagnostic {
    fn from(diagnostic: Diagnostic<'_>) -> Self {
        match diagnostic {
            Diagnostic::TokenHash => Self::TokenHash,
            Diagnostic::CachedPlan { message } => Self::CachedPlan(message.to_owned()),
            Diagnostic::AttemptFailure { message } => Self::AttemptFailure(message.to_owned()),
            Diagnostic::FailureTraceback { message } => Self::FailureTraceback(message.to_owned()),
            Diagnostic::Backoff { tries } => Self::Backoff(tries),
            Diagnostic::DeprecatedUsed => Self::DeprecatedUsed,
            Diagnostic::DeprecatedLookupFailed { message } => {
                Self::DeprecatedLookupFailed(message.to_owned())
            }
        }
    }
}

pub(crate) struct RecordingDiagnostics {
    pub(crate) seen: Mutex<Vec<ObservedDiagnostic>>,
    pub(crate) timeline: Timeline,
}

impl RecordingDiagnostics {
    pub(crate) fn shared(timeline: &Timeline) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            timeline: Arc::clone(timeline),
        }
    }
}

impl RecordingDiagnostics {
    pub(crate) fn seen(&self) -> Vec<ObservedDiagnostic> {
        lock(&self.seen).clone()
    }
}

impl KeyReadDiagnostics for RecordingDiagnostics {
    fn emit(&self, diagnostic: Diagnostic<'_>) {
        let label = match &diagnostic {
            Diagnostic::TokenHash => "hash",
            Diagnostic::CachedPlan { .. } => "cached-plan",
            Diagnostic::AttemptFailure { .. } => "failure",
            Diagnostic::FailureTraceback { .. } => "traceback",
            Diagnostic::Backoff { .. } => "backoff",
            Diagnostic::DeprecatedUsed => "deprecated-used",
            Diagnostic::DeprecatedLookupFailed { .. } => "deprecated-failed",
        };
        lock(&self.timeline).push(label.to_owned());
        lock(&self.seen).push(ObservedDiagnostic::from(diagnostic));
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedHook {
    pub(crate) call_type: &'static str,
    pub(crate) class: &'static str,
    pub(crate) message: String,
    pub(crate) traceback: String,
    pub(crate) duration: Duration,
    pub(crate) failure: KeyReadError,
}

pub(crate) struct RecordingHook {
    pub(crate) hooks: Mutex<Vec<RecordedHook>>,
    pub(crate) timeline: Timeline,
}

impl RecordingHook {
    pub(crate) fn shared(timeline: &Timeline) -> Self {
        Self {
            hooks: Mutex::new(Vec::new()),
            timeline: Arc::clone(timeline),
        }
    }
}

impl RecordingHook {
    pub(crate) fn hooks(&self) -> Vec<RecordedHook> {
        lock(&self.hooks).clone()
    }
}

impl KeyFailureHook for RecordingHook {
    fn schedule(&self, hook: FailureHook) -> HookFuture {
        let class = hook.failure.class();
        lock(&self.timeline).push(String::from("hook"));
        lock(&self.hooks).push(RecordedHook {
            call_type: hook.call_type,
            class,
            message: hook.message,
            traceback: hook.traceback,
            duration: hook.duration,
            failure: hook.failure,
        });
        Box::pin(async {})
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the metric sink failed")]
pub(crate) struct SinkFailure;

pub(crate) struct RecordingSink {
    pub(crate) success_calls: Mutex<Vec<String>>,
    pub(crate) failure_calls: Mutex<Vec<(String, ErrorType)>>,
    pub(crate) timeline: Timeline,
}

impl RecordingSink {
    pub(crate) fn shared(timeline: &Timeline) -> Self {
        Self {
            success_calls: Mutex::new(Vec::new()),
            failure_calls: Mutex::new(Vec::new()),
            timeline: Arc::clone(timeline),
        }
    }
}

impl DbServiceEventSink for RecordingSink {
    type Failure = SinkFailure;

    fn success(&self, event: DbServiceEvent) -> DbMetricFuture<'_, Self::Failure> {
        lock(&self.timeline).push(String::from("metric-success"));
        lock(&self.success_calls).push(event.call_type);
        Box::pin(async { Ok(()) })
    }

    fn failure(&self, event: DbServiceFailureEvent) -> DbMetricFuture<'_, Self::Failure> {
        lock(&self.timeline).push(String::from("metric-failure"));
        lock(&self.failure_calls).push((event.event.call_type, event.error.error_type));
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedRowCall {
    pub(crate) hash: String,
    pub(crate) use_writer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedGraceCall {
    pub(crate) hash: String,
    pub(crate) use_writer: bool,
}

#[derive(Default)]
pub(crate) struct ScriptedDatabase {
    pub(crate) rows: Mutex<VecDeque<Result<Option<JsonData>, DatabaseFailure>>>,
    pub(crate) deprecated: Mutex<VecDeque<Result<Option<DeprecatedToken>, DatabaseFailure>>>,
    pub(crate) row_calls: Mutex<Vec<RecordedRowCall>>,
    pub(crate) grace_calls: Mutex<Vec<RecordedGraceCall>>,
    pub(crate) timeline: Timeline,
}

impl ScriptedDatabase {
    pub(crate) fn shared(timeline: &Timeline) -> Self {
        let mut database = Self::default();
        database.timeline = Arc::clone(timeline);
        database
    }
}

impl ScriptedDatabase {
    pub(crate) fn row_calls(&self) -> Vec<RecordedRowCall> {
        lock(&self.row_calls).clone()
    }

    pub(crate) fn grace_calls(&self) -> Vec<RecordedGraceCall> {
        lock(&self.grace_calls).clone()
    }

    pub(crate) fn push_row(&self, row: Result<Option<JsonData>, DbError>) {
        lock(&self.rows).push_back(row.map_err(DatabaseFailure::from));
    }

    pub(crate) fn push_grace(&self, row: Result<Option<DeprecatedToken>, DbError>) {
        lock(&self.deprecated).push_back(row.map_err(DatabaseFailure::from));
    }
}

fn pop_step<T>(steps: &Mutex<VecDeque<T>>) -> Option<T> {
    let mut queue = lock(steps);
    queue.pop_front()
}

impl KeyReadDatabase for ScriptedDatabase {
    fn read_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        use_writer: bool,
    ) -> RowFuture<'lookup> {
        let recorded = RecordedRowCall {
            hash: hash.expose_secret().to_owned(),
            use_writer,
        };
        lock(&self.row_calls).push(recorded);
        lock(&self.timeline).push(String::from("read"));
        let next = pop_step(&self.rows).unwrap_or(Ok(None));
        Box::pin(async move { next })
    }

    fn deprecated_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        now: DateTime<Utc>,
        use_writer: bool,
    ) -> DeprecatedFuture<'lookup> {
        let next = pop_step(&self.deprecated).unwrap_or(Ok(None));
        let next = match next {
            Ok(Some(row)) if row.revoke_at <= now => Ok(None),
            other => other,
        };
        let recorded = RecordedGraceCall {
            hash: hash.expose_secret().to_owned(),
            use_writer,
        };
        lock(&self.grace_calls).push(recorded);
        Box::pin(async move { next })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedReconnect {
    pub(crate) reason: String,
    pub(crate) force: bool,
    pub(crate) force_recreate: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) lock_timeout: Option<Duration>,
    pub(crate) has_database_url: bool,
    pub(crate) stale_is_live: bool,
}

#[derive(Default)]
pub(crate) struct ScriptedReconnect {
    pub(crate) generation: u64,
    pub(crate) handle: Option<Arc<u64>>,
    pub(crate) last_observed: Mutex<u64>,
    pub(crate) results: Mutex<VecDeque<Result<bool, ReconnectFailure>>>,
    pub(crate) requests: Mutex<Vec<RecordedReconnect>>,
    pub(crate) timeline: Timeline,
}

impl ScriptedReconnect {
    pub(crate) fn shared(generation: u64, handle: Option<Arc<u64>>, timeline: &Timeline) -> Self {
        let mut reconnect = Self::default();
        reconnect.generation = generation;
        reconnect.handle = handle;
        reconnect.last_observed = Mutex::new(generation);
        reconnect.timeline = Arc::clone(timeline);
        reconnect
    }

    pub(crate) fn requests(&self) -> Vec<RecordedReconnect> {
        lock(&self.requests).clone()
    }

    pub(crate) fn push_result(&self, result: Result<bool, ReconnectFailure>) {
        lock(&self.results).push_back(result);
    }
}

impl PlanReconnect for ScriptedReconnect {
    type Handle = u64;

    fn read_target(&self) -> Observation<u64> {
        lock(&self.timeline).push(String::from("observe"));
        *lock(&self.last_observed) = self.generation;
        Observation {
            generation: self.generation,
            handle: self.handle.clone(),
            connected: self.handle.is_some(),
        }
    }

    fn attempt<'reconnect>(
        &'reconnect self,
        request: &'reconnect ReconnectRequest<'_>,
        stale: Option<&'reconnect StaleReadEngine<Self::Handle>>,
    ) -> ReconnectFuture<'reconnect> {
        lock(&self.timeline).push(String::from("reconnect"));
        let observed = Observation {
            generation: *lock(&self.last_observed),
            handle: self.handle.clone(),
            connected: self.handle.is_some(),
        };
        lock(&self.requests).push(RecordedReconnect {
            reason: request.reason.to_owned(),
            force: request.force,
            force_recreate: request.force_recreate,
            timeout: request.timeout,
            lock_timeout: request.lock_timeout,
            has_database_url: request.database_url.is_some(),
            stale_is_live: stale.is_some_and(|engine| engine.is_still_live(&observed)),
        });
        let next = lock(&self.results).pop_front().unwrap_or(Ok(true));
        Box::pin(async move { next })
    }
}

pub(crate) struct Harness {
    pub(crate) database: ScriptedDatabase,
    pub(crate) reconnect: ScriptedReconnect,
    pub(crate) sink: Arc<RecordingSink>,
    pub(crate) clock: StepClock,
    pub(crate) jitter: FullJitter,
    pub(crate) pause: RecordingPause,
    pub(crate) diagnostics: RecordingDiagnostics,
    pub(crate) hook: RecordingHook,
    pub(crate) tasks: TaskTracker,
    pub(crate) cache: Arc<Mutex<DeprecatedKeyCache>>,
    pub(crate) timeline: Timeline,
}

impl Harness {
    pub(crate) fn new(step: Duration) -> Self {
        let timeline = Timeline::default();
        Self {
            database: ScriptedDatabase::shared(&timeline),
            reconnect: ScriptedReconnect::shared(1, Some(Arc::new(7)), &timeline),
            sink: Arc::new(RecordingSink::shared(&timeline)),
            clock: StepClock::new(step, fixed_now()),
            jitter: FullJitter,
            pause: RecordingPause::shared(&timeline),
            diagnostics: RecordingDiagnostics::shared(&timeline),
            hook: RecordingHook::shared(&timeline),
            tasks: TaskTracker::new(),
            cache: Arc::new(Mutex::new(DeprecatedKeyCache::default())),
            timeline,
        }
    }

    pub(crate) async fn read(
        &self,
        request: &CombinedKeyRequest,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let reader = KeyReader {
            database: &self.database,
            reconnect: &self.reconnect,
            sink: Arc::clone(&self.sink),
            services: ReadServices {
                clock: &self.clock,
                jitter: &self.jitter,
                pause: &self.pause,
                diagnostics: &self.diagnostics,
                hook: &self.hook,
                resolver: &NoSpanResolver,
                tasks: &self.tasks,
            },
            cache: Arc::clone(&self.cache),
        };
        reader.read(request).await
    }

    pub(crate) fn pauses(&self) -> Vec<Duration> {
        lock(&self.pause.delays).clone()
    }
}

pub(crate) fn fault(error_type: ErrorType, message: &str) -> DbError {
    DbError {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

pub(crate) fn cached_plan_fault() -> DbError {
    fault(
        ErrorType::RawQuery,
        "ERROR: cached plan must not change result type",
    )
}

pub(crate) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(crate) fn number(value: i64) -> JsonData {
    JsonData::Number(serde_json::Number::from(value))
}

pub(crate) fn decimal(value: &str) -> JsonData {
    JsonData::Number(serde_json::from_str(value).unwrap())
}

pub(crate) fn view_fields(pairs: Vec<(&str, JsonData)>) -> BTreeMap<String, JsonData> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(crate) fn object(pairs: Vec<(&str, JsonData)>) -> JsonData {
    JsonData::Object(view_fields(pairs))
}

pub(crate) fn token_row(token: &str) -> BTreeMap<String, JsonData> {
    view_fields(vec![
        ("token", text(token)),
        ("models", JsonData::Array(Vec::new())),
        ("spend", decimal("0")),
        ("total_spend", decimal("0")),
    ])
}

pub(crate) fn view_row(token: &str) -> Option<JsonData> {
    Some(JsonData::Object(token_row(token)))
}

pub(crate) fn grace_hit(active: &str, revoke_at: DateTime<Utc>) -> DeprecatedToken {
    DeprecatedToken {
        active_token_id: SecretString::from(active.to_owned()),
        revoke_at,
    }
}
