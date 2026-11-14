use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_db::auth_reconnect::clock::ReconnectClock;
use betterlite_db::auth_reconnect::lock::ReconnectLock;
use betterlite_db::database_refresh::handle::PoolFuture;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::fakes::{Effect, Effects, FakeError, record};
use super::{LockMode, LockScript};

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
    pub(super) fn from_values(values: &[f64]) -> Result<Self, Box<crate::helpers::HelperError>> {
        let mut offsets = VecDeque::new();
        for value in values {
            let offset = Duration::try_from_secs_f64(*value).map_err(|_error| {
                Box::new(crate::helpers::HelperError {
                    class: "ValueError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: "clock values are nonnegative seconds".to_owned(),
                })
            })?;
            offsets.push_back(offset);
        }
        let fallback = offsets.back().copied().unwrap_or_default();
        Ok(Self {
            base: Instant::now(),
            script: Arc::new(Mutex::new(ClockScript { offsets, fallback })),
        })
    }

    pub(super) fn at(&self, offset: Duration) -> Instant {
        self.base + offset
    }
}

impl ReconnectClock for ScriptedClock {
    fn now(&self) -> Instant {
        let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        let fallback = script.fallback;
        self.base + script.offsets.pop_front().unwrap_or(fallback)
    }
}

struct LockShared {
    script: LockScript,
    effects: Effects,
}

#[derive(Clone)]
pub(super) struct ScriptedLock {
    shared: Arc<LockShared>,
}

impl ScriptedLock {
    pub(super) fn new(script: &LockScript, effects: Effects) -> Self {
        Self {
            shared: Arc::new(LockShared {
                script: script.clone(),
                effects,
            }),
        }
    }
}

pub(super) struct ScriptedHold {
    effects: Effects,
}

impl Drop for ScriptedHold {
    fn drop(&mut self) {
        record(&self.effects, Effect::LockRelease);
    }
}

impl ReconnectLock for ScriptedLock {
    type Error = FakeError;
    type Hold = ScriptedHold;

    fn acquire(&self, _limit: Option<Duration>) -> PoolFuture<'_, Option<Self::Hold>, Self::Error> {
        Box::pin(async move {
            record(&self.shared.effects, Effect::LockAcquire);
            match self.shared.script.mode {
                LockMode::Free => Ok(Some(ScriptedHold {
                    effects: Arc::clone(&self.shared.effects),
                })),
                LockMode::Busy => Ok(None),
                LockMode::Error => {
                    Err(FakeError(self.shared.script.error.clone().unwrap_or_else(
                        || "the reconnect lock is unavailable".to_owned(),
                    )))
                }
            }
        })
    }
}

pub(super) struct Capture {
    effects: Effects,
}

impl Capture {
    pub(super) fn new(effects: Effects) -> Self {
        Self { effects }
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

fn level_name(level: tracing::Level) -> &'static str {
    if level == tracing::Level::DEBUG {
        "debug"
    } else if level == tracing::Level::INFO {
        "info"
    } else if level == tracing::Level::WARN {
        "warn"
    } else if level == tracing::Level::ERROR {
        "error"
    } else {
        "trace"
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut fields = Fields(BTreeMap::new());
        event.record(&mut fields);
        record(
            &self.effects,
            Effect::Log {
                level: level_name(*event.metadata().level()),
                fields: fields.0,
            },
        );
    }
}
