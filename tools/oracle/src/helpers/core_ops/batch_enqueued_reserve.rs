use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedPrincipal;
use betterlite_core::api::batch_enqueued_reserve::store::{
    EnqueuedTokenCounters, LocalTokenCounters, ReservationScripts, ReserveScriptReply,
};
use betterlite_core::api::batch_enqueued_reserve::{
    BATCH_ENQUEUED_TTL_SECONDS, BatchEnqueuedReserveInput, BatchEnqueuedReserveResult,
    BatchEnqueuedTokenScope, BatchEnqueuedTokenSettings, BatchEnqueuedTokenStore,
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
    tokens: i64,
    principal: BatchEnqueuedPrincipal,
    redis: Option<ScriptsInput>,
    memory: MemoryInput,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptsInput {
    reserve: Vec<Step>,
    refund: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Step {
    Failure(String),
    Integer(i64),
    Reply(Vec<i64>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MemoryInput {
    seed: BTreeMap<String, i64>,
    read: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct ScriptFailure {
    message: String,
}

struct ScriptedScripts {
    events: Arc<Mutex<Vec<Value>>>,
    reserve: Mutex<VecDeque<Step>>,
    refund: Mutex<VecDeque<Step>>,
}

impl ScriptedScripts {
    fn new(events: Arc<Mutex<Vec<Value>>>, spec: ScriptsInput) -> Self {
        Self {
            events,
            reserve: Mutex::new(VecDeque::from(spec.reserve)),
            refund: Mutex::new(VecDeque::from(spec.refund)),
        }
    }

    fn record(&self, event: Value) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn next_step(&self, queue: &Mutex<VecDeque<Step>>, event: Value) -> Option<Step> {
        self.record(event);
        queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }

    fn refund_reply(&self, counter_key: &str, tokens: i64) -> Result<(), ScriptFailure> {
        let step = self
            .refund
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        self.record(json!({
            "event": "refund",
            "key": counter_key,
            "args": [tokens],
        }));
        match step {
            Some(Step::Integer(_) | Step::Reply(_)) => Ok(()),
            Some(Step::Failure(message)) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted refund queue is empty".to_owned(),
            }),
        }
    }
}

fn reserve_reply(step: Option<Step>) -> Result<ReserveScriptReply, ScriptFailure> {
    let Some(next) = step else {
        return Err(ScriptFailure {
            message: "the scripted reserve queue is empty".to_owned(),
        });
    };
    match next {
        Step::Reply(values) => match values.as_slice() {
            [status, count] => Ok(ReserveScriptReply {
                status: *status,
                count: *count,
            }),
            _ => Err(ScriptFailure {
                message: "the scripted reserve reply is not a status pair".to_owned(),
            }),
        },
        Step::Integer(value) => Err(ScriptFailure {
            message: format!("the scripted reserve reply {value} is not a status pair"),
        }),
        Step::Failure(message) => Err(ScriptFailure { message }),
    }
}

impl ReservationScripts for ScriptedScripts {
    type Error = ScriptFailure;

    fn reserve(
        &self,
        counter_key: &str,
        tokens: i64,
        ttl_seconds: i64,
        limit: i64,
    ) -> impl Future<Output = Result<ReserveScriptReply, ScriptFailure>> + Send {
        let event = json!({
            "event": "reserve",
            "key": counter_key,
            "args": [tokens, ttl_seconds, limit],
        });
        let step = self.next_step(&self.reserve, event);
        ready(reserve_reply(step))
    }

    fn refund(
        &self,
        counter_key: &str,
        tokens: i64,
    ) -> impl Future<Output = Result<(), ScriptFailure>> + Send {
        ready(self.refund_reply(counter_key, tokens))
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BatchEnqueuedReserveError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn scope_value(scope: &BatchEnqueuedTokenScope) -> Value {
    json!({
        "key": scope.key,
        "value": scope.value,
        "limit": scope.limit,
    })
}

fn outcome_value(outcome: &BatchEnqueuedReserveResult) -> Value {
    match outcome {
        BatchEnqueuedReserveResult::Reservation(reserved) => json!({
            "kind": "reservation",
            "tokens": reserved.tokens,
            "scopes": reserved.scopes.iter().map(scope_value).collect::<Vec<_>>(),
            "backend": reserved.backend,
            "owner_present": !reserved.owner.is_empty(),
        }),
        BatchEnqueuedReserveResult::OverLimit(found) => json!({
            "kind": "over_limit",
            "scope": scope_value(&found.scope),
            "enqueued": found.enqueued,
        }),
    }
}

fn seeded_counters(
    memory: &MemoryInput,
) -> Result<Arc<Mutex<LocalTokenCounters>>, Box<HelperError>> {
    let cache = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(|error| failure(error.to_string()))?;
    let counters = Arc::new(Mutex::new(LocalTokenCounters { cache }));
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    for (key, value) in &memory.seed {
        EnqueuedTokenCounters::set(&mut *held, key, *value, BATCH_ENQUEUED_TTL_SECONDS)
            .map_err(|error| failure(error.to_string()))?;
    }
    drop(held);
    Ok(counters)
}

fn read_memory(
    counters: &Arc<Mutex<LocalTokenCounters>>,
    keys: &[String],
) -> Result<Map<String, Value>, Box<HelperError>> {
    let mut memory = Map::new();
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    for key in keys {
        let stored = EnqueuedTokenCounters::get(&mut *held, key)
            .map_err(|error| failure(error.to_string()))?;
        memory.insert(key.clone(), stored.map_or(Value::Null, Value::from));
    }
    drop(held);
    Ok(memory)
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
        let store = BatchEnqueuedTokenStore {
            clock: Arc::new(|| 0.0_f64),
            scripts,
            counters: Arc::clone(&counters),
            owner: OWNER.to_owned(),
            settings: BatchEnqueuedTokenSettings::default(),
        };
        let record = BatchEnqueuedReserveInput {
            tokens: input.tokens,
            principal: input.principal.clone(),
        };
        let guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(capture::Capture(Arc::clone(&events))),
        );
        let result = runtime
            .block_on(store.reserve(&record))
            .map_err(|error| failure(error.to_string()));
        drop(guard);
        let outcome = result?;
        let memory = read_memory(&counters, &input.memory.read)?;
        let captured = events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({
            "outcome": outcome_value(&outcome),
            "events": captured,
            "memory": memory,
        }))
    }
}
