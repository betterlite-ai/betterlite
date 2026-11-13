use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_refund::store::{
    EnqueuedLocalStore, ReservationRecordScripts,
};
use betterlite_core::api::batch_enqueued_refund::{
    BatchEnqueuedRefundError, BatchEnqueuedRefundInput, BatchEnqueuedRefundResult,
    EnqueuedTokenRefund,
};
use betterlite_core::api::batch_enqueued_reserve::store::{
    EnqueuedTokenCounters, LocalTokenCounters, ReservationScripts, ReserveScriptReply,
};
use betterlite_core::api::batch_enqueued_reserve::{
    BATCH_ENQUEUED_TTL_SECONDS, BatchEnqueuedTokenScope, BatchEnqueuedTokenSettings,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

mod capture;
mod tests;

const OWNER: &str = "oracle-worker";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    plan: BatchEnqueuedRefundInput,
    #[serde(default)]
    redis: Option<ScriptsInput>,
    #[serde(default)]
    memory: MemoryInput,
    #[serde(default)]
    now: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptsInput {
    #[serde(default)]
    refund: Vec<Step>,
    #[serde(default)]
    save: Vec<Step>,
    #[serde(default)]
    pop: Vec<PopStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Done,
    Failure { message: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PopStep {
    Record { record: Value },
    Tombstone,
    Absent,
    Failure { message: String },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MemoryInput {
    counters: BTreeMap<String, i64>,
    records: BTreeMap<String, String>,
    read: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted script call failed: {message}")]
struct ScriptFailure {
    message: String,
}

struct ScriptedScripts {
    events: Arc<Mutex<Vec<Value>>>,
    refunds: Mutex<VecDeque<Step>>,
    saves: Mutex<VecDeque<Step>>,
    pops: Mutex<VecDeque<PopStep>>,
}

impl ScriptedScripts {
    fn new(events: Arc<Mutex<Vec<Value>>>, spec: ScriptsInput) -> Self {
        Self {
            events,
            refunds: Mutex::new(VecDeque::from(spec.refund)),
            saves: Mutex::new(VecDeque::from(spec.save)),
            pops: Mutex::new(VecDeque::from(spec.pop)),
        }
    }

    fn record(&self, event: Value) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn take_step(
        &self,
        queue: &Mutex<VecDeque<Step>>,
        event: Value,
    ) -> Result<(), ScriptFailure> {
        self.record(event);
        let step = queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match step {
            Some(Step::Done) => Ok(()),
            Some(Step::Failure { message }) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted queue is empty".to_owned(),
            }),
        }
    }
}

impl ReservationScripts for ScriptedScripts {
    type Error = ScriptFailure;

    fn reserve(
        &self,
        _counter_key: &str,
        _tokens: i64,
        _ttl_seconds: i64,
        _limit: i64,
    ) -> impl Future<Output = Result<ReserveScriptReply, ScriptFailure>> + Send {
        ready(Err(ScriptFailure {
            message: "the refund helper never reserves".to_owned(),
        }))
    }

    fn refund(
        &self,
        counter_key: &str,
        tokens: i64,
    ) -> impl Future<Output = Result<(), ScriptFailure>> + Send {
        let event = json!({"event": "refund", "key": counter_key, "args": [tokens]});
        ready(self.take_step(&self.refunds, event))
    }
}

impl ReservationRecordScripts for ScriptedScripts {
    fn save(
        &self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<(), ScriptFailure>> + Send {
        let stored: Value = serde_json::from_str(record).unwrap_or_else(|_error| Value::Null);
        let event = json!({
            "event": "save",
            "key": record_key,
            "record": stored,
            "args": [ttl_seconds],
        });
        ready(self.take_step(&self.saves, event))
    }

    fn pop(
        &self,
        record_key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, ScriptFailure>> + Send {
        self.push_event(json!({"event": "pop", "key": record_key, "args": [ttl_seconds]}));
        let step = self
            .pops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let outcome = match step {
            Some(PopStep::Record { record }) => serde_json::to_vec(&record)
                .map(Some)
                .map_err(|error| ScriptFailure {
                    message: error.to_string(),
                }),
            Some(PopStep::Tombstone) => Ok(Some(Vec::new())),
            Some(PopStep::Absent) => Ok(None),
            Some(PopStep::Failure { message }) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted pop queue is empty".to_owned(),
            }),
        };
        ready(outcome)
    }
}

struct RecordingStore {
    counters: LocalTokenCounters,
    events: Arc<Mutex<Vec<Value>>>,
}

impl RecordingStore {
    fn push_event(&self, event: Value) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

impl EnqueuedLocalStore for RecordingStore {
    fn counter(&mut self, counter_key: &str) -> Result<i64, BatchEnqueuedRefundError> {
        EnqueuedLocalStore::counter(&mut self.counters, counter_key)
    }

    fn set_counter(
        &mut self,
        counter_key: &str,
        value: i64,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError> {
        let outcome =
            EnqueuedLocalStore::set_counter(&mut self.counters, counter_key, value, ttl_seconds);
        if outcome.is_ok() {
            self.push_event(json!({
                "event": "local_set",
                "key": counter_key,
                "value": value,
                "args": [ttl_seconds],
            }));
        }
        outcome
    }

    fn delete(&mut self, key: &str) {
        self.push_event(json!({"event": "local_delete", "key": key}));
        EnqueuedLocalStore::delete(&mut self.counters, key);
    }

    fn record(&mut self, record_key: &str) -> Result<Option<Vec<u8>>, BatchEnqueuedRefundError> {
        EnqueuedLocalStore::record(&mut self.counters, record_key)
    }

    fn set_record(
        &mut self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError> {
        let outcome =
            EnqueuedLocalStore::set_record(&mut self.counters, record_key, record, ttl_seconds);
        if outcome.is_ok() {
            let stored: Value = serde_json::from_str(record).unwrap_or_else(|_error| Value::Null);
            self.push_event(json!({
                "event": "local_set",
                "key": record_key,
                "value": stored,
                "args": [ttl_seconds],
            }));
        }
        outcome
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BatchEnqueuedRefundError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn seeded_counters(memory: &MemoryInput) -> Result<LocalTokenCounters, Box<HelperError>> {
    let cache = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(|error| failure(error.to_string()))?;
    let mut counters = LocalTokenCounters { cache };
    let seconds = u64::try_from(BATCH_ENQUEUED_TTL_SECONDS)
        .map_err(|error| failure(error.to_string()))?;
    let ttl = Duration::from_secs(seconds).as_secs_f64();
    for (key, value) in &memory.counters {
        EnqueuedTokenCounters::set(&mut counters, key, *value, BATCH_ENQUEUED_TTL_SECONDS)
            .map_err(|error| failure(error.to_string()))?;
    }
    for (key, text) in &memory.records {
        counters
            .cache
            .set(key, CacheValue::Text(text.clone()), Some(ttl))
            .map_err(|error| failure(error.to_string()))?;
    }
    Ok(counters)
}

fn cached_value(value: CacheValue) -> Value {
    match value {
        CacheValue::Text(text) => match serde_json::from_str::<Value>(&text) {
            Ok(parsed) => parsed,
            Err(_error) => Value::String(text),
        },
        other => serde_json::to_value(other).unwrap_or_else(|_error| Value::Null),
    }
}

fn read_memory(
    store: &Arc<Mutex<RecordingStore>>,
    keys: &[String],
) -> Result<Map<String, Value>, Box<HelperError>> {
    let mut store = store.lock().unwrap_or_else(PoisonError::into_inner);
    let mut memory = Map::new();
    for key in keys {
        let stored = store
            .counters
            .cache
            .get(key)
            .map_err(|error| failure(error.to_string()))?;
        memory.insert(key.clone(), stored.map_or(Value::Null, cached_value));
    }
    Ok(memory)
}

fn scope_value(scope: &BatchEnqueuedTokenScope) -> Value {
    json!({
        "key": scope.key,
        "value": scope.value,
        "limit": scope.limit,
    })
}

fn record_value(record: &ReservationRecord) -> Value {
    json!({
        "tokens": record.tokens,
        "scopes": record.scopes.iter().map(scope_value).collect::<Vec<_>>(),
        "backend": record.backend,
        "owner": record.owner,
        "reserved_at_monotonic": record.reserved_at_monotonic,
    })
}

fn outcome_value(result: &BatchEnqueuedRefundResult) -> Value {
    match result {
        BatchEnqueuedRefundResult::PostCall(outcome) => json!({
            "kind": "post_call",
            "batch_id": outcome.batch_id,
            "saved": outcome.saved,
            "refunded": outcome.refunded,
        }),
        BatchEnqueuedRefundResult::Saved => json!({"kind": "saved"}),
        BatchEnqueuedRefundResult::Popped(reservation) => json!({
            "kind": "popped",
            "reservation": reservation.as_ref().map(record_value),
        }),
        BatchEnqueuedRefundResult::Refunded => json!({"kind": "refunded"}),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let scripts = input
            .redis
            .clone()
            .map(|spec| ScriptedScripts::new(Arc::clone(&events), spec));
        let counters = seeded_counters(&input.memory)?;
        let store = Arc::new(Mutex::new(RecordingStore {
            counters,
            events: Arc::clone(&events),
        }));
        let now = input.now;
        let service = EnqueuedTokenRefund {
            clock: Arc::new(move || now),
            scripts,
            store: Arc::clone(&store),
            owner: OWNER.to_owned(),
            settings: BatchEnqueuedTokenSettings::default(),
            limits: ApiLimits::default(),
        };
        let guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(capture::CaptureLayer(Arc::clone(&events))),
        );
        let result = runtime
            .block_on(service.run(&input.plan))
            .map_err(|error| failure(error.to_string()));
        drop(guard);
        let outcome = outcome_value(&result?);
        let memory = read_memory(&store, &input.memory.read)?;
        let captured = events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({
            "outcome": outcome,
            "events": captured,
            "memory": memory,
        }))
    }
}
