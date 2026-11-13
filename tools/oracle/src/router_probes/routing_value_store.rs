mod parser;
mod steps;

use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

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
    pub default_ttl_millis: Option<u64>,
    pub max_size_per_item: Option<i64>,
}

// integer milliseconds, since a tagged float cannot be read back under serde_json's arbitrary_precision
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Set {
        now_millis: u64,
        key: String,
        value: RouteValue,
        ttl_millis: Option<u64>,
    },
    SetPipeline {
        now_millis: u64,
        entries: Vec<Entry>,
        ttl_millis: Option<u64>,
    },
    SetAdd {
        now_millis: u64,
        key: String,
        values: Vec<String>,
        ttl_millis: Option<u64>,
    },
    Get {
        now_millis: u64,
        key: String,
    },
    BatchGet {
        now_millis: u64,
        keys: Vec<String>,
    },
    Increment {
        now_millis: u64,
        key: String,
        by: RouteIncrement,
        ttl_millis: Option<u64>,
    },
    CompareAndSet {
        now_millis: u64,
        key: String,
        expected: Option<RouteValue>,
        value: RouteValue,
        ttl_millis: Option<u64>,
    },
    Parse {
        payload: ParsePayload,
    },
}

impl Step {
    const fn clock(&self) -> Option<u64> {
        match self {
            Self::Set { now_millis, .. }
            | Self::SetPipeline { now_millis, .. }
            | Self::SetAdd { now_millis, .. }
            | Self::Get { now_millis, .. }
            | Self::BatchGet { now_millis, .. }
            | Self::Increment { now_millis, .. }
            | Self::CompareAndSet { now_millis, .. } => Some(*now_millis),
            Self::Parse { .. } => None,
        }
    }

    fn call<'store>(&self, store: &'store RoutingValueStore) -> Call<'store> {
        let (key, ttl_millis) = match self {
            Self::Set {
                key, ttl_millis, ..
            }
            | Self::SetAdd {
                key, ttl_millis, ..
            }
            | Self::Increment {
                key, ttl_millis, ..
            }
            | Self::CompareAndSet {
                key, ttl_millis, ..
            } => (Some(key), *ttl_millis),
            Self::SetPipeline { ttl_millis, .. } => (None, *ttl_millis),
            Self::Get { key, .. } => (Some(key), None),
            Self::BatchGet { .. } | Self::Parse { .. } => (None, None),
        };
        Call {
            store,
            key: key.and_then(|raw| RouteKey::parse(raw).ok()),
            ttl: ttl_millis.map(|millis| Duration::from_millis(millis).as_secs_f64()),
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
            default_ttl_seconds: input
                .settings
                .default_ttl_millis
                .map(|millis| Duration::from_millis(millis).as_secs_f64()),
            max_size_per_item: input.settings.max_size_per_item,
            loaded_max_size_per_item_kib: 1024,
        },
        clock,
    )?;
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        if let Some(millis) = step.clock() {
            *now.lock().unwrap_or_else(PoisonError::into_inner) =
                Duration::from_millis(millis).as_secs_f64();
        }
        steps.push(execute_step(&store, step)?);
    }
    Ok(ProbeOutput::RoutingValueStore(Output { steps }))
}

fn execute_step(store: &RoutingValueStore, step: &Step) -> StepResult {
    match step {
        Step::Set { value, .. } => steps::execute_set(&step.call(store), value),
        Step::SetPipeline { entries, .. } => {
            steps::execute_set_pipeline(&step.call(store), entries)
        }
        Step::SetAdd { values, .. } => steps::execute_set_add(&step.call(store), values),
        Step::Get { key, .. } => Ok(StepOutput::Get {
            value: steps::read_one(store, key)?,
        }),
        Step::BatchGet { keys, .. } => steps::execute_batch_get(store, keys),
        Step::Increment { by, .. } => steps::execute_increment(&step.call(store), by),
        Step::CompareAndSet {
            expected, value, ..
        } => steps::execute_compare_and_set(&step.call(store), expected.as_ref(), value),
        Step::Parse { payload } => Ok(StepOutput::Parse {
            result: parser::parse_payload(payload)?,
        }),
    }
}
