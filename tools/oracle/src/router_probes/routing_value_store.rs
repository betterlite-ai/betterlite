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
    )
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })?;
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        if let Some(at) = step.clock() {
            *now.lock().unwrap_or_else(PoisonError::into_inner) = at.0;
        }
        steps.push(execute_step(&store, step)?);
    }
    Ok(ProbeOutput::RoutingValueStore(Output { steps }))
}

fn execute_step(store: &RoutingValueStore, step: &Step) -> Result<StepOutput, OperationError> {
    match step {
        Step::Set {
            key, value, ttl, ..
        } => execute_set(store, key, value, *ttl),
        Step::SetPipeline { entries, ttl, .. } => execute_set_pipeline(store, entries, *ttl),
        Step::SetAdd {
            key, values, ttl, ..
        } => execute_set_add(store, key, values, *ttl),
        Step::Get { key, .. } => Ok(StepOutput::Get {
            value: read_one(store, key)?,
        }),
        Step::BatchGet { keys, .. } => execute_batch_get(store, keys),
        Step::Increment { key, by, ttl, .. } => execute_increment(store, key, by, *ttl),
        Step::CompareAndSet {
            key,
            expected,
            value,
            ttl,
            ..
        } => execute_compare_and_set(store, key, expected.as_ref(), value, *ttl),
        Step::Parse { payload } => Ok(StepOutput::Parse {
            result: parse_payload(payload)?,
        }),
    }
}

fn execute_set(
    store: &RoutingValueStore,
    key: &str,
    value: &RouteValue,
    ttl: Option<Seconds>,
) -> Result<StepOutput, OperationError> {
    let Ok(route_key) = RouteKey::parse(key) else {
        return Ok(StepOutput::Set);
    };
    store
        .set(&route_key, value, ttl.map(|seconds| seconds.0))
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok(StepOutput::Set)
}

fn execute_set_pipeline(
    store: &RoutingValueStore,
    entries: &[Entry],
    ttl: Option<Seconds>,
) -> Result<StepOutput, OperationError> {
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let Ok(key) = RouteKey::parse(&entry.key) else {
            continue;
        };
        parsed.push((key, entry.value.clone()));
    }
    store
        .set_pipeline(&parsed, ttl.map(|seconds| seconds.0))
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok(StepOutput::SetPipeline)
}

fn execute_set_add(
    store: &RoutingValueStore,
    key: &str,
    values: &[String],
    ttl: Option<Seconds>,
) -> Result<StepOutput, OperationError> {
    let Ok(route_key) = RouteKey::parse(key) else {
        return Ok(StepOutput::SetAdd {
            values: values.to_vec(),
        });
    };
    let added = store
        .set_add(&route_key, values, ttl.map(|seconds| seconds.0))
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok(StepOutput::SetAdd { values: added })
}

fn read_one(store: &RoutingValueStore, key: &str) -> Result<Option<RouteValue>, OperationError> {
    let Ok(route_key) = RouteKey::parse(key) else {
        return Ok(None);
    };
    let read = store
        .get(&route_key)
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok(read_value(&read))
}

fn execute_batch_get(
    store: &RoutingValueStore,
    keys: &[String],
) -> Result<StepOutput, OperationError> {
    let values = keys
        .iter()
        .map(|key| read_one(store, key))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StepOutput::BatchGet { values })
}

fn execute_increment(
    store: &RoutingValueStore,
    key: &str,
    by: &RouteIncrement,
    ttl: Option<Seconds>,
) -> Result<StepOutput, OperationError> {
    let route_key = RouteKey::parse(key).map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })?;
    let value = store
        .increment(&route_key, by, ttl.map(|seconds| seconds.0))
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok(StepOutput::Increment { value })
}

fn execute_compare_and_set(
    store: &RoutingValueStore,
    key: &str,
    expected: Option<&RouteValue>,
    value: &RouteValue,
    ttl: Option<Seconds>,
) -> Result<StepOutput, OperationError> {
    let route_key = RouteKey::parse(key).map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })?;
    let wanted = expected.map_or(StoreRead::Missing, |current| {
        StoreRead::Value(current.clone())
    });
    let outcome = store
        .compare_and_set(&route_key, &wanted, value, ttl.map(|seconds| seconds.0))
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok(compare_output(&outcome))
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
            serde_json::to_vec(&value).map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?
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
        Err(error) => Err(OperationError::InvalidRequest {
            message: error.to_string(),
        }),
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
