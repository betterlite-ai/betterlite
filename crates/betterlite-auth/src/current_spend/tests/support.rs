use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::budget_primitives::window_start::DurationParser;
use crate::current_spend::cache::{CounterBatch, CounterCache};
use crate::model_budget_limiter::clock::SystemClock as UptimeClock;
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, SpendRow, SpendRowLookup, WindowEntity, WindowSpendRow,
};
use crate::spend_reseed::values::ReseedCacheValue;

pub(super) type Journal = Arc<Mutex<Vec<String>>>;
pub(super) type Memory = Arc<Mutex<BTreeMap<String, ReseedCacheValue>>>;

pub(super) fn money(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a fixture amount")
}

pub(super) fn entries<T: Clone>(shared: &Mutex<Vec<T>>) -> Vec<T> {
    shared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn window_start() -> Timestamp {
    let moment = NaiveDate::from_ymd_opt(2026, 11, 18)
        .and_then(|day| day.and_hms_opt(12, 0, 0))
        .expect("a fixture moment");
    Timestamp::Naive(moment)
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted store failed")]
pub(super) struct ScriptFailure;

#[derive(Clone)]
pub(super) enum Script<Value> {
    Answer(Value),
    Fail,
}

impl<Value: Clone> Script<Value> {
    fn play(&self) -> Result<Value, ScriptFailure> {
        match self {
            Self::Answer(value) => Ok(value.clone()),
            Self::Fail => Err(ScriptFailure),
        }
    }
}

pub(super) struct ScriptedRedis {
    pub(super) get: Script<Option<ReseedCacheValue>>,
    pub(super) set_max: Script<Option<Decimal>>,
}

pub(super) struct ScriptedCache {
    pub(super) redis: Option<ScriptedRedis>,
    pub(super) memory: Memory,
    pub(super) expiring: Mutex<Vec<(String, u32)>>,
    pub(super) journal: Journal,
}

impl ScriptedCache {
    pub(super) fn stored(&self, key: &str) -> Option<ReseedCacheValue> {
        let memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        memory.get(key).cloned()
    }

    fn put(&self, key: &str, value: Decimal) {
        let mut memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        memory.insert(key.to_owned(), ReseedCacheValue::Number(value));
    }
}

impl ReseedCache for ScriptedCache {
    type Error = ScriptFailure;

    fn redis_configured(&self) -> bool {
        self.redis.is_some()
    }

    fn redis_get(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, ScriptFailure>> + Send {
        ready(
            self.redis
                .as_ref()
                .map_or(Ok(None), |redis| redis.get.play()),
        )
    }

    fn redis_set_nx(
        &self,
        _counter_key: &str,
        _value: Decimal,
    ) -> impl Future<Output = Result<Option<bool>, ScriptFailure>> + Send {
        ready(Ok(self.redis.as_ref().map(|_redis| true)))
    }

    fn memory_get(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, ScriptFailure>> + Send {
        ready(Ok(self.stored(counter_key)))
    }

    fn memory_set(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<(), ScriptFailure>> + Send {
        self.put(counter_key, value);
        ready(Ok(()))
    }

    fn increment_local(
        &self,
        _counter_key: &str,
        _increment: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, ScriptFailure>> + Send {
        ready(Ok(None))
    }
}

impl CounterCache for ScriptedCache {
    fn memory_set_expiring(
        &self,
        key: &str,
        value: Decimal,
        ttl_seconds: u32,
    ) -> Result<(), ScriptFailure> {
        self.put(key, value);
        let mut expiring = self.expiring.lock().unwrap_or_else(PoisonError::into_inner);
        expiring.push((key.to_owned(), ttl_seconds));
        Ok(())
    }

    fn redis_set_max(
        &self,
        counter_key: &str,
        _value: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, ScriptFailure>> + Send {
        let mut journal = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        journal.push(format!("set_max {counter_key}"));
        drop(journal);
        let answer = self
            .redis
            .as_ref()
            .map_or(Ok(None), |redis| redis.set_max.play());
        ready(answer)
    }
}

pub(super) struct JournalBatch {
    pub(super) answer: Mutex<Option<CounterValue>>,
    pub(super) recorded: Mutex<Vec<(String, Decimal)>>,
    pub(super) journal: Journal,
}

impl ActiveSpendBatch for JournalBatch {
    fn read(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<CounterValue>, SpendCounterBatchError>> + Send {
        ready(Ok(*self
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)))
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        let mut recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        recorded.push((counter_key.to_owned(), value));
    }
}

impl CounterBatch for JournalBatch {
    fn forget(&self, counter_key: &str) {
        let mut journal = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        journal.push(format!("forget {counter_key}"));
        *self.answer.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

pub(super) struct ScriptedRows {
    pub(super) primary: Option<Decimal>,
    pub(super) end_user: Option<Decimal>,
    pub(super) window: Option<Decimal>,
    pub(super) fails: bool,
    pub(super) lookups: Journal,
}

fn stored_row(spend: Option<Decimal>) -> Option<SpendRow> {
    spend.map(|amount| SpendRow {
        spend: Some(JsonData::String(amount.to_string())),
    })
}

impl ScriptedRows {
    fn note(&self, entry: String) -> Result<(), ScriptFailure> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
        if self.fails {
            return Err(ScriptFailure);
        }
        Ok(())
    }
}

impl SpendCounterRows for ScriptedRows {
    type Error = ScriptFailure;

    fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<SpendRow>, ScriptFailure>> + Send {
        let spend = match lookup {
            SpendRowLookup::EndUser(_) => self.end_user,
            SpendRowLookup::Key(_)
            | SpendRowLookup::TeamMember { .. }
            | SpendRowLookup::Team(_)
            | SpendRowLookup::User(_)
            | SpendRowLookup::Organization(_)
            | SpendRowLookup::Project(_) => self.primary,
        };
        ready(self.note(format!("{lookup:?}")).map(|()| stored_row(spend)))
    }

    fn window_row(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, ScriptFailure>> + Send {
        let noted = self.note(format!("window {entity:?} {entity_id} {window_duration}"));
        ready(noted.map(|()| {
            self.window.map(|amount| WindowSpendRow {
                window_start: window_start(),
                spend: Some(JsonData::String(amount.to_string())),
            })
        }))
    }

    fn window_log_rows(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        _window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, ScriptFailure>> + Send {
        ready(
            self.note(format!("logs {entity:?} {entity_id}"))
                .map(|()| Vec::new()),
        )
    }
}

struct NoDurations;

impl DurationParser for NoDurations {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

pub(super) fn reseed_over<Rows: SpendCounterRows>(rows: Option<Rows>) -> SpendCounterReseed<Rows> {
    SpendCounterReseed {
        rows: rows.map(Arc::new),
        locks: LockRegistry::from(10_000_i64),
        clock: Box::new(UptimeClock {
            started: Instant::now(),
        }),
        durations: Box::new(NoDurations),
        tasks: TaskTracker::new(),
        gate: Arc::new(LoopBoundSemaphore {
            value: 25,
            slot: Mutex::new(None),
        }),
        stall_tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        deadline_seconds: 5.0_f64,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoggedEvent {
    pub(super) level: Level,
    pub(super) message: String,
    pub(super) counter_key: Option<String>,
}

#[derive(Default)]
struct EventFields {
    message: String,
    counter_key: Option<String>,
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => value.clone_into(&mut self.message),
            "counter_key" => self.counter_key = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct EventLog(pub(super) Arc<Mutex<Vec<LoggedEvent>>>);

impl<S: Subscriber> Layer<S> for EventLog {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggedEvent {
                level: *event.metadata().level(),
                message: fields.message,
                counter_key: fields.counter_key,
            });
    }
}

pub(super) fn capture_events() -> (EventLog, DefaultGuard) {
    let log = EventLog::default();
    let guard = set_default(registry().with(log.clone()));
    (log, guard)
}
