mod parser;
mod steps;

use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use betterlite_router::routing_value_store::{
    Clock, RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError, StoreSettings,
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

impl From<StoreError> for OperationError {
    fn from(error: StoreError) -> Self {
        Self::InvalidRequest {
            message: error.to_string(),
        }
    }
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
        } => steps::execute_set(&route_call(store, Some(key.as_str()), *ttl), value),
        Step::SetPipeline { entries, ttl, .. } => {
            steps::execute_set_pipeline(&route_call(store, None, *ttl), entries)
        }
        Step::SetAdd {
            key, values, ttl, ..
        } => steps::execute_set_add(&route_call(store, Some(key.as_str()), *ttl), values),
        Step::Get { key, .. } => Ok(StepOutput::Get {
            value: steps::read_one(store, key)?,
        }),
        Step::BatchGet { keys, .. } => steps::execute_batch_get(store, keys),
        Step::Increment { key, by, ttl, .. } => {
            steps::execute_increment(&route_call(store, Some(key.as_str()), *ttl), by)
        }
        Step::CompareAndSet {
            key,
            expected,
            value,
            ttl,
            ..
        } => steps::execute_compare_and_set(
            &route_call(store, Some(key.as_str()), *ttl),
            expected.as_ref(),
            value,
        ),
        Step::Parse { payload } => Ok(StepOutput::Parse {
            result: parser::parse_payload(payload)?,
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
