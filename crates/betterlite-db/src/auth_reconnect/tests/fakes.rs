use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use secrecy::SecretString;
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

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
pub(super) enum Step<T> {
    Done(T),
    Failure(String),
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadRoute {
    Writer,
    Reader,
}

pub(super) struct EngineScript {
    pub(super) writer_generation: u64,
    pub(super) reader_generation: u64,
    pub(super) read_route: ReadRoute,
    pub(super) engine_state: EngineState,
    pub(super) probes: VecDeque<Step<bool>>,
    pub(super) recreates: VecDeque<Step<bool>>,
    pub(super) selects: VecDeque<Step<()>>,
    pub(super) rearms: VecDeque<Step<()>>,
    pub(super) bump_on_probe: bool,
    pub(super) hold_recreate: Option<Arc<Notify>>,
}

impl Default for EngineScript {
    fn default() -> Self {
        Self {
            writer_generation: 0,
            reader_generation: 0,
            read_route: ReadRoute::Writer,
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

    fn with_script<T, F>(&self, select: F) -> T
    where
        F: FnOnce(&mut EngineScript) -> T,
    {
        let mut script = self
            .shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        select(&mut script)
    }

    fn observation(&self, handle: Arc<Handle>, generation: u64) -> Observation<Handle> {
        Observation {
            generation,
            handle: Some(handle),
            connected: true,
        }
    }

    async fn settle<F>(&self, effect: Effect, next: F) -> Result<(), FakeError>
    where
        F: FnOnce(&mut EngineScript) -> Option<Step<()>>,
    {
        record(&self.shared.effects, effect);
        apply(self.with_script(next).unwrap_or(Step::Done(()))).await
    }

    pub(super) fn writer_handle(&self) -> Arc<Handle> {
        Arc::clone(&self.shared.writer)
    }

    pub(super) fn reader_handle(&self) -> Arc<Handle> {
        Arc::clone(&self.shared.reader)
    }

    pub(super) fn writer_observation(&self) -> Observation<Handle> {
        let generation = self.with_script(|script| script.writer_generation);
        self.observation(Arc::clone(&self.shared.writer), generation)
    }

    pub(super) fn reader_observation(&self) -> Observation<Handle> {
        let generation = self.with_script(|script| script.reader_generation);
        self.observation(Arc::clone(&self.shared.reader), generation)
    }
}

async fn apply<T>(step: Step<T>) -> Result<T, FakeError> {
    match step {
        Step::Done(value) => Ok(value),
        Step::Failure(message) => Err(FakeError(message)),
        Step::Pending => pending().await,
    }
}

impl ReconnectEngine for FakeEngine {
    type Handle = Handle;
    type Error = FakeError;

    fn writer(&self) -> Observation<Self::Handle> {
        self.writer_observation()
    }

    fn read_target(&self) -> Observation<Self::Handle> {
        let (handle, generation) = self.with_script(|script| {
            if script.read_route == ReadRoute::Reader {
                (Arc::clone(&self.shared.reader), script.reader_generation)
            } else {
                (Arc::clone(&self.shared.writer), script.writer_generation)
            }
        });
        self.observation(handle, generation)
    }

    fn engine_state(&self) -> EngineState {
        self.with_script(|script| script.engine_state)
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
            let gate = self.with_script(|script| script.hold_recreate.clone());
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let step = self
                .with_script(|script| script.recreates.pop_front())
                .unwrap_or(Step::Done(true));
            apply(step).await
        })
    }

    fn writer_read_only(&self) -> PoolFuture<'_, bool, Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::Probe);
            let step = self.with_script(|script| {
                if script.bump_on_probe {
                    script.writer_generation = script.writer_generation.saturating_add(1);
                }
                script.probes.pop_front()
            });
            apply(step.unwrap_or(Step::Done(false))).await
        })
    }

    fn writer_select_one(&self) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(self.settle(Effect::SelectOne, |script| script.selects.pop_front()))
    }

    fn mark_writer_recovered(&self) {
        let read_route = self.with_script(|script| script.read_route);
        if read_route == ReadRoute::Reader {
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
        Box::pin(self.settle(Effect::Rearm, |script| script.rearms.pop_front()))
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
        let offsets: VecDeque<Duration> = values
            .iter()
            .map(|value| {
                Duration::try_from_secs_f64(*value)
                    .expect("scripted clock values are nonnegative seconds")
            })
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
        let offset = script.offsets.pop_front().unwrap_or(fallback);
        self.base.checked_add(offset).unwrap_or(self.base)
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
    effects: Effects,
}

#[derive(Clone)]
pub(super) struct FakeLock {
    shared: Arc<LockShared>,
}

impl FakeLock {
    pub(super) fn new(mode: LockMode, effects: Effects) -> Self {
        Self {
            shared: Arc::new(LockShared { mode, effects }),
        }
    }
}

pub(super) struct FakeHold {
    shared: Arc<LockShared>,
}

impl Drop for FakeHold {
    fn drop(&mut self) {
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
                LockMode::Free => Ok(Some(FakeHold {
                    shared: Arc::clone(&self.shared),
                })),
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
