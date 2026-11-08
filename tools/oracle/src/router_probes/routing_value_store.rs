use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use betterlite_router::routing_value_store::codec::parse_route_value;
use betterlite_router::routing_value_store::{
    Clock, CompareOutcome, MAX_VALUE_BYTES, PinValue, RouteIncrement, RouteKey, RouteValue,
    RoutingValueStore, StoreError, StoreRead, StoreSettings,
};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

type StepResult = Result<StepOutput, OperationError>;
type ReadResult = Result<Option<RouteValue>, OperationError>;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub settings: SettingsInput,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub max_size_in_memory: Option<i64>,
    pub default_ttl_seconds: Option<Seconds>,
    pub max_size_per_item: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seconds(pub f64);

impl Eq for Seconds {}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Set {
        now: Seconds,
        key: String,
        value: RouteValue,
        ttl: Option<Seconds>,
    },
    SetPipeline {
        now: Seconds,
        entries: Vec<Entry>,
        ttl: Option<Seconds>,
    },
    SetAdd {
        now: Seconds,
        key: String,
        values: Vec<String>,
        ttl: Option<Seconds>,
    },
    Get {
        now: Seconds,
        key: String,
    },
    BatchGet {
        now: Seconds,
        keys: Vec<String>,
    },
    Increment {
        now: Seconds,
        key: String,
        by: RouteIncrement,
        ttl: Option<Seconds>,
    },
    CompareAndSet {
        now: Seconds,
        key: String,
        expected: Option<RouteValue>,
        value: RouteValue,
        ttl: Option<Seconds>,
    },
    Parse {
        payload: ParsePayload,
    },
}

impl Step {
    const fn clock(&self) -> Option<Seconds> {
        match self {
            Self::Set { now, .. }
            | Self::SetPipeline { now, .. }
            | Self::SetAdd { now, .. }
            | Self::Get { now, .. }
            | Self::BatchGet { now, .. }
            | Self::Increment { now, .. }
            | Self::CompareAndSet { now, .. } => Some(*now),
            Self::Parse { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub key: String,
    pub value: RouteValue,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParsePayload {
    Text { text: String },
    Pin { model_id_bytes: u64 },
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepOutput {
    Set,
    SetPipeline,
    SetAdd {
        values: Vec<String>,
    },
    Get {
        value: Option<RouteValue>,
    },
    BatchGet {
        values: Vec<Option<RouteValue>>,
    },
    Increment {
        value: RouteValue,
    },
    CompareAndSet {
        swapped: bool,
        current: Option<RouteValue>,
    },
    Parse {
        result: ParseResult,
    },
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParseResult {
    Valid { value: ValueSummary },
    Invalid { bytes: usize, limit: usize },
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueSummary {
    Counter {
        value: i64,
    },
    Spend {
        amount: String,
    },
    Pin {
        model_id_bytes: usize,
    },
    SchedulerTuple {
        priority: u8,
        request_id_bytes: usize,
    },
    Members {
        members: Vec<String>,
    },
}

struct Call<'store> {
    store: &'store RoutingValueStore,
    key: Option<RouteKey>,
    ttl: Option<f64>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let now = Arc::new(StdMutex::new(0.0_f64));
    let clock_source = Arc::clone(&now);
    let clock: Clock =
        Arc::new(move || *clock_source.lock().unwrap_or_else(PoisonError::into_inner));
    let store = RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: input.settings.max_size_in_memory,
            default_ttl_seconds: input.settings.default_ttl_seconds.map(|seconds| seconds.0),
            max_size_per_item: input.settings.max_size_per_item,
            loaded_max_size_per_item_kib: 1024,
        },
        clock,
    )?;
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        if let Some(at) = step.clock() {
            *now.lock().unwrap_or_else(PoisonError::into_inner) = at.0;
        }
        steps.push(execute_step(&store, step)?);
    }
    Ok(ProbeOutput::RoutingValueStore(Output { steps }))
}

fn execute_step(store: &RoutingValueStore, step: &Step) -> StepResult {
    match step {
        Step::Set {
            key, value, ttl, ..
        } => execute_set(&route_call(store, Some(key.as_str()), *ttl), value),
        Step::SetPipeline { entries, ttl, .. } => {
            execute_set_pipeline(&route_call(store, None, *ttl), entries)
        }
        Step::SetAdd {
            key, values, ttl, ..
        } => execute_set_add(&route_call(store, Some(key.as_str()), *ttl), values),
        Step::Get { key, .. } => Ok(StepOutput::Get {
            value: read_one(store, key)?,
        }),
        Step::BatchGet { keys, .. } => execute_batch_get(store, keys),
        Step::Increment { key, by, ttl, .. } => {
            execute_increment(&route_call(store, Some(key.as_str()), *ttl), by)
        }
        Step::CompareAndSet {
            key,
            expected,
            value,
            ttl,
            ..
        } => execute_compare_and_set(
            &route_call(store, Some(key.as_str()), *ttl),
            expected.as_ref(),
            value,
        ),
        Step::Parse { payload } => Ok(StepOutput::Parse {
            result: parse_payload(payload)?,
        }),
    }
}

fn route_call<'store>(
    store: &'store RoutingValueStore,
    key: Option<&str>,
    ttl: Option<Seconds>,
) -> Call<'store> {
    Call {
        store,
        key: key.and_then(|raw| RouteKey::parse(raw).ok()),
        ttl: ttl.map(|seconds| seconds.0),
    }
}

fn execute_set(call: &Call<'_>, value: &RouteValue) -> StepResult {
    if let Some(route_key) = &call.key {
        call.store.set(route_key, value, call.ttl)?;
    }
    Ok(StepOutput::Set)
}

fn execute_set_pipeline(call: &Call<'_>, entries: &[Entry]) -> StepResult {
    let parsed = entries.iter().filter_map(entry_key).collect::<Vec<_>>();
    call.store.set_pipeline(&parsed, call.ttl)?;
    Ok(StepOutput::SetPipeline)
}

fn entry_key(entry: &Entry) -> Option<(RouteKey, RouteValue)> {
    RouteKey::parse(&entry.key)
        .ok()
        .map(|key| (key, entry.value.clone()))
}

fn execute_set_add(call: &Call<'_>, values: &[String]) -> StepResult {
    let added = match &call.key {
        Some(route_key) => call.store.set_add(route_key, values, call.ttl)?,
        None => values.to_vec(),
    };
    Ok(StepOutput::SetAdd { values: added })
}

fn read_one(store: &RoutingValueStore, raw: &str) -> ReadResult {
    let Ok(route_key) = RouteKey::parse(raw) else {
        return Ok(None);
    };
    let read = store.get(&route_key)?;
    Ok(read_value(&read))
}

fn execute_batch_get(store: &RoutingValueStore, keys: &[String]) -> StepResult {
    let values = keys
        .iter()
        .map(|key| read_one(store, key))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StepOutput::BatchGet { values })
}

fn execute_increment(call: &Call<'_>, by: &RouteIncrement) -> StepResult {
    let route_key = call
        .key
        .as_ref()
        .ok_or_else(|| missing_key("an increment"))?;
    let value = call.store.increment(route_key, by, call.ttl)?;
    Ok(StepOutput::Increment { value })
}

fn execute_compare_and_set(
    call: &Call<'_>,
    expected: Option<&RouteValue>,
    value: &RouteValue,
) -> StepResult {
    let route_key = call
        .key
        .as_ref()
        .ok_or_else(|| missing_key("a compare and set"))?;
    let wanted = expected.map_or(StoreRead::Missing, |current| {
        StoreRead::Value(current.clone())
    });
    let outcome = call
        .store
        .compare_and_set(route_key, &wanted, value, call.ttl)?;
    Ok(compare_output(&outcome))
}

impl From<StoreError> for OperationError {
    fn from(error: StoreError) -> Self {
        Self::InvalidRequest {
            message: error.to_string(),
        }
    }
}

fn missing_key(operation: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: format!("{operation} needs a route key within the key limit"),
    }
}

fn compare_output(outcome: &CompareOutcome) -> StepOutput {
    StepOutput::CompareAndSet {
        swapped: outcome.swapped,
        current: read_value(&outcome.current),
    }
}

fn read_value(read: &StoreRead) -> Option<RouteValue> {
    match read {
        StoreRead::Missing => None,
        StoreRead::Value(value) => Some(value.clone()),
    }
}

fn parse_payload(payload: &ParsePayload) -> Result<ParseResult, OperationError> {
    let bytes = match payload {
        ParsePayload::Text { text } => text.as_bytes().to_vec(),
        ParsePayload::Pin { model_id_bytes } => {
            let length = usize::try_from(*model_id_bytes).map_err(|error| {
                OperationError::InvalidRequest {
                    message: error.to_string(),
                }
            })?;
            if length > MAX_VALUE_BYTES.saturating_add(1) {
                return Err(OperationError::InvalidRequest {
                    message: "the pin model id length is past the parser bound".to_owned(),
                });
            }
            let value = RouteValue::Pin(PinValue {
                model_id: "a".repeat(length),
            });
            serde_json::to_vec(&value).map_err(|source| StoreError::Encoding { source })?
        }
    };
    match parse_route_value(&bytes) {
        Ok(value) => Ok(ParseResult::Valid {
            value: summarize(&value),
        }),
        Err(StoreError::ValueTooLarge {
            bytes: reported,
            limit,
        }) => Ok(ParseResult::Invalid {
            bytes: reported,
            limit,
        }),
        Err(error) => Err(OperationError::from(error)),
    }
}

fn summarize(value: &RouteValue) -> ValueSummary {
    match value {
        RouteValue::Counter(counter) => ValueSummary::Counter { value: *counter },
        RouteValue::Spend(spend) => ValueSummary::Spend {
            amount: spend.to_string(),
        },
        RouteValue::Pin(pin) => ValueSummary::Pin {
            model_id_bytes: pin.model_id.len(),
        },
        RouteValue::SchedulerTuple(tuple) => ValueSummary::SchedulerTuple {
            priority: tuple.priority,
            request_id_bytes: tuple.request_id.len(),
        },
        RouteValue::Members(members) => ValueSummary::Members {
            members: members.iter().cloned().collect(),
        },
    }
}
