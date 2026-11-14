use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use secrecy::SecretString;
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::auth_reconnect::clock::ReconnectClock;
use crate::auth_reconnect::engine::{EngineState, ReconnectEngine};
use crate::auth_reconnect::lock::ReconnectLock;
use crate::database_refresh::generation::Observation;
use crate::database_refresh::handle::PoolFuture;

pub(super) struct Handle;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct FakeError(pub(super) String);

pub(super) type Effects = Arc<Mutex<Vec<Effect>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Effect {
    Probe,
    Recreate { expected_generation: Option<u64> },
    SelectOne,
    MarkWriterRecovered,
    Reap,
    Cleanup,
    Rearm,
    LockAcquire,
    LockRelease,
}

pub(super) fn effects() -> Effects {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn recorded(effects: &Effects) -> Vec<Effect> {
    effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn record(effects: &Effects, effect: Effect) {
    effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(effect);
}

#[derive(Debug, Clone)]
pub(super) enum ProbeOutcome {
    Value(bool),
    Failure(String),
    Pending,
}

#[derive(Debug, Clone)]
pub(super) enum RecreateOutcome {
    Value(bool),
    Failure(String),
    Pending,
}

#[derive(Debug, Clone)]
pub(super) enum StepOutcome {
    Done,
    Failure(String),
    Pending,
}

pub(super) struct EngineScript {
    pub(super) writer_generation: u64,
    pub(super) reader_generation: u64,
    pub(super) reader_unavailable: bool,
    pub(super) routing: bool,
    pub(super) engine_state: EngineState,
    pub(super) probes: VecDeque<ProbeOutcome>,
    pub(super) recreates: VecDeque<RecreateOutcome>,
    pub(super) selects: VecDeque<StepOutcome>,
    pub(super) rearms: VecDeque<StepOutcome>,
    pub(super) bump_on_probe: bool,
    pub(super) hold_recreate: Option<Arc<Notify>>,
}

impl Default for EngineScript {
    fn default() -> Self {
        Self {
            writer_generation: 0,
            reader_generation: 0,
            reader_unavailable: false,
            routing: false,
            engine_state: EngineState::Alive,
            probes: VecDeque::new(),
            recreates: VecDeque::new(),
            selects: VecDeque::new(),
            rearms: VecDeque::new(),
            bump_on_probe: false,
            hold_recreate: None,
        }
    }
}

struct EngineShared {
    writer: Arc<Handle>,
    reader: Arc<Handle>,
    script: Mutex<EngineScript>,
    effects: Effects,
}

#[derive(Clone)]
pub(super) struct FakeEngine {
    shared: Arc<EngineShared>,
}

impl FakeEngine {
    pub(super) fn new(script: EngineScript, effects: Effects) -> Self {
        Self {
            shared: Arc::new(EngineShared {
                writer: Arc::new(Handle),
                reader: Arc::new(Handle),
                script: Mutex::new(script),
                effects,
            }),
        }
    }

    pub(super) fn writer_handle(&self) -> Arc<Handle> {
        Arc::clone(&self.shared.writer)
    }

    pub(super) fn reader_handle(&self) -> Arc<Handle> {
        Arc::clone(&self.shared.reader)
    }

    pub(super) fn writer_observation(&self) -> Observation<Handle> {
        let script = self
            .shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Observation {
            generation: script.writer_generation,
            handle: Some(Arc::clone(&self.shared.writer)),
            connected: true,
        }
    }

    pub(super) fn reader_observation(&self) -> Observation<Handle> {
        let script = self
            .shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Observation {
            generation: script.reader_generation,
            handle: Some(Arc::clone(&self.shared.reader)),
            connected: true,
        }
    }

    pub(super) fn set_engine_state(&self, state: EngineState) {
        self.shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .engine_state = state;
    }
}

async fn step_outcome(outcome: StepOutcome) -> Result<(), FakeError> {
    match outcome {
        StepOutcome::Done => Ok(()),
        StepOutcome::Failure(message) => Err(FakeError(message)),
        StepOutcome::Pending => std::future::pending::<Result<(), FakeError>>().await,
    }
}

impl ReconnectEngine for FakeEngine {
    type Handle = Handle;
    type Error = FakeError;

    fn writer(&self) -> Observation<Self::Handle> {
        self.writer_observation()
    }

    fn read_target(&self) -> Observation<Self::Handle> {
        let script = self
            .shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if script.routing && !script.reader_unavailable {
            Observation {
                generation: script.reader_generation,
                handle: Some(Arc::clone(&self.shared.reader)),
                connected: true,
            }
        } else {
            Observation {
                generation: script.writer_generation,
                handle: Some(Arc::clone(&self.shared.writer)),
                connected: true,
            }
        }
    }

    fn engine_state(&self) -> EngineState {
        self.shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .engine_state
    }

    fn recreate(
        &self,
        _url: &SecretString,
        expected_generation: Option<u64>,
    ) -> PoolFuture<'_, bool, Self::Error> {
        Box::pin(async move {
            record(
                &self.shared.effects,
                Effect::Recreate {
                    expected_generation,
                },
            );
            let gate = self
                .shared
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .hold_recreate
                .clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let outcome = self
                .shared
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recreates
                .pop_front()
                .unwrap_or(RecreateOutcome::Value(true));
            match outcome {
                RecreateOutcome::Value(value) => Ok(value),
                RecreateOutcome::Failure(message) => Err(FakeError(message)),
                RecreateOutcome::Pending => std::future::pending::<Result<bool, FakeError>>().await,
            }
        })
    }

    fn writer_read_only(&self) -> PoolFuture<'_, bool, Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::Probe);
            let outcome = {
                let mut script = self
                    .shared
                    .script
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if script.bump_on_probe {
                    script.writer_generation = script.writer_generation.saturating_add(1);
                }
                script
                    .probes
                    .pop_front()
                    .unwrap_or(ProbeOutcome::Value(false))
            };
            match outcome {
                ProbeOutcome::Value(value) => Ok(value),
                ProbeOutcome::Failure(message) => Err(FakeError(message)),
                ProbeOutcome::Pending => std::future::pending::<Result<bool, FakeError>>().await,
            }
        })
    }

    fn writer_select_one(&self) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::SelectOne);
            let outcome = self
                .shared
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .selects
                .pop_front()
                .unwrap_or(StepOutcome::Done);
            step_outcome(outcome).await
        })
    }

    fn mark_writer_recovered(&self) {
        let routing = self
            .shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .routing;
        if routing {
            record(&self.shared.effects, Effect::MarkWriterRecovered);
        }
    }

    fn reap(&self) {
        record(&self.shared.effects, Effect::Reap);
    }

    fn cleanup_watcher(&self) {
        record(&self.shared.effects, Effect::Cleanup);
    }

    fn rearm_watcher(&self) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::Rearm);
            let outcome = self
                .shared
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .rearms
                .pop_front()
                .unwrap_or(StepOutcome::Done);
            step_outcome(outcome).await
        })
    }
}

struct ClockScript {
    offsets: VecDeque<Duration>,
    fallback: Duration,
}

#[derive(Clone)]
pub(super) struct ScriptedClock {
    base: Instant,
    script: Arc<Mutex<ClockScript>>,
}

impl ScriptedClock {
    pub(super) fn from_seconds(values: &[f64]) -> Self {
        let mut offsets: VecDeque<Duration> = values
            .iter()
            .map(|value| Duration::from_secs_f64(*value))
            .collect();
        let fallback = offsets.back().copied().unwrap_or_default();
        Self {
            base: Instant::now(),
            script: Arc::new(Mutex::new(ClockScript { offsets, fallback })),
        }
    }
}

impl ReconnectClock for ScriptedClock {
    fn now(&self) -> Instant {
        let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        let fallback = script.fallback;
        self.base + script.offsets.pop_front().unwrap_or(fallback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LockMode {
    Free,
    Timeout,
    Error,
}

struct LockShared {
    mode: LockMode,
    held: AtomicBool,
    effects: Effects,
}

#[derive(Clone)]
pub(super) struct FakeLock {
    shared: Arc<LockShared>,
}

impl FakeLock {
    pub(super) fn new(mode: LockMode, effects: Effects) -> Self {
        Self {
            shared: Arc::new(LockShared {
                mode,
                held: AtomicBool::new(false),
                effects,
            }),
        }
    }

    pub(super) fn held(&self) -> bool {
        self.shared.held.load(Ordering::SeqCst)
    }
}

pub(super) struct FakeHold {
    shared: Arc<LockShared>,
}

impl Drop for FakeHold {
    fn drop(&mut self) {
        self.shared.held.store(false, Ordering::SeqCst);
        record(&self.shared.effects, Effect::LockRelease);
    }
}

impl ReconnectLock for FakeLock {
    type Error = FakeError;
    type Hold = FakeHold;

    fn acquire(&self, _limit: Option<Duration>) -> PoolFuture<'_, Option<Self::Hold>, Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::LockAcquire);
            match self.shared.mode {
                LockMode::Free => {
                    self.shared.held.store(true, Ordering::SeqCst);
                    Ok(Some(FakeHold {
                        shared: Arc::clone(&self.shared),
                    }))
                }
                LockMode::Timeout => Ok(None),
                LockMode::Error => Err(FakeError("the reconnect lock is unavailable".to_owned())),
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LogEntry {
    pub(super) level: tracing::Level,
    pub(super) fields: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<LogEntry>>>);

impl CapturedLogs {
    pub(super) fn entries(&self) -> Vec<LogEntry> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() != "message" {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() != "message" {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut fields = Fields(BTreeMap::new());
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogEntry {
                level: *event.metadata().level(),
                fields: fields.0,
            });
    }
}

pub(super) fn capture<T, F>(run: F) -> (T, Vec<LogEntry>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, run);
    (value, logs.entries())
}
