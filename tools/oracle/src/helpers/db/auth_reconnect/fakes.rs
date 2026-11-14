use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::auth_reconnect::engine::{EngineState, ReconnectEngine};
use betterlite_db::database_refresh::generation::Observation;
use betterlite_db::database_refresh::handle::PoolFuture;
use secrecy::SecretString;
use serde::Serialize;

use super::{Outcome, TargetName};

pub(super) struct Handle;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct FakeError(pub(super) String);

pub(super) type Effects = Arc<Mutex<Vec<Effect>>>;

pub(super) fn effects() -> Effects {
    Arc::new(Mutex::new(Vec::new()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub(super) enum Effect {
    LockAcquire,
    LockRelease,
    Recreate {
        expected_generation: Option<u64>,
    },
    Probe,
    SelectOne,
    MarkWriterRecovered,
    Rearm,
    Cleanup,
    Log {
        level: &'static str,
        fields: BTreeMap<String, String>,
    },
}

pub(super) fn record(effects: &Effects, effect: Effect) {
    effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(effect);
}

pub(super) struct EngineScript {
    pub(super) writer_generation: u64,
    pub(super) reader_generation: u64,
    pub(super) routing: bool,
    pub(super) reader_unavailable: bool,
    pub(super) engine_state: EngineState,
    pub(super) probe: Outcome,
    pub(super) recreate: Outcome,
    pub(super) select: Outcome,
    pub(super) bump_on_probe: bool,
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

    pub(super) fn observation(&self, target: TargetName, generation: u64) -> Observation<Handle> {
        let routing = self
            .shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .routing;
        let handle = if target == TargetName::Writer || !routing {
            Arc::clone(&self.shared.writer)
        } else {
            Arc::clone(&self.shared.reader)
        };
        Observation {
            generation,
            handle: Some(handle),
            connected: true,
        }
    }

    pub(super) fn current_generation(&self, target: TargetName) -> u64 {
        let script = self
            .shared
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if target == TargetName::Reader {
            script.reader_generation
        } else {
            script.writer_generation
        }
    }

    pub(super) fn report_handle(&self, target: TargetName) -> Arc<Handle> {
        match target {
            TargetName::Writer => Arc::clone(&self.shared.writer),
            TargetName::Reader => Arc::clone(&self.shared.reader),
        }
    }
}

fn outcome(scripted: &Outcome, default: bool) -> Result<bool, FakeError> {
    if let Some(error) = &scripted.error {
        return Err(FakeError(error.clone()));
    }
    Ok(scripted.value.unwrap_or(default))
}

impl ReconnectEngine for FakeEngine {
    type Handle = Handle;
    type Error = FakeError;

    fn writer(&self) -> Observation<Self::Handle> {
        let generation = self.current_generation(TargetName::Writer);
        self.observation(TargetName::Writer, generation)
    }

    fn read_target(&self) -> Observation<Self::Handle> {
        let target = {
            let script = self
                .shared
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if script.routing && !script.reader_unavailable {
                TargetName::Reader
            } else {
                TargetName::Writer
            }
        };
        let generation = self.current_generation(target);
        self.observation(target, generation)
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
            let scripted = self
                .shared
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recreate
                .clone();
            if scripted.pending {
                return std::future::pending::<Result<bool, FakeError>>().await;
            }
            outcome(&scripted, true)
        })
    }

    fn writer_read_only(&self) -> PoolFuture<'_, bool, Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::Probe);
            let scripted = {
                let mut script = self
                    .shared
                    .script
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if script.bump_on_probe {
                    script.writer_generation = script.writer_generation.saturating_add(1);
                }
                script.probe.clone()
            };
            if scripted.pending {
                return std::future::pending::<Result<bool, FakeError>>().await;
            }
            outcome(&scripted, false)
        })
    }

    fn writer_select_one(&self) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::SelectOne);
            let scripted = self
                .shared
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .select
                .clone();
            if scripted.pending {
                return std::future::pending::<Result<(), FakeError>>().await;
            }
            match scripted.error {
                Some(error) => Err(FakeError(error)),
                None => Ok(()),
            }
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

    fn reap(&self) {}

    fn cleanup_watcher(&self) {
        record(&self.shared.effects, Effect::Cleanup);
    }

    fn rearm_watcher(&self) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::Rearm);
            Ok(())
        })
    }
}
