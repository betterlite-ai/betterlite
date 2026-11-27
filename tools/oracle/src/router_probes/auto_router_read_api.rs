use std::fmt::Display;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod steps;
mod store;

use self::steps::{StepOutcome, benchmarks_step, session_step};

// the enums in mod.rs would otherwise carry every field of this payload inline in their largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    now_unix_millis: i64,
    #[serde(default)]
    deployments: Vec<Deployment>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Benchmarks(BenchmarksStep),
    Session(SessionStep),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BenchmarksStep {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    database: bool,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    end_date: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    rows: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionStep {
    #[serde(default)]
    api_key: String,
    session_id: String,
    #[serde(default)]
    sessions: Vec<SessionSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionSeed {
    last_turn_at: String,
    row: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    steps: Vec<StepOutcome>,
}

fn invalid<T>(error: &T) -> OperationError
where
    T: Display + ?Sized,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

async fn scenario(payload: &Payload) -> Result<Fields, OperationError> {
    let today = DateTime::<Utc>::from_timestamp_millis(payload.now_unix_millis)
        .map(|instant| instant.date_naive())
        .ok_or_else(|| invalid("the probe clock is outside the supported calendar"))?;
    let mut outcomes = Vec::with_capacity(payload.steps.len());
    for step in &payload.steps {
        outcomes.push(match step {
            Step::Benchmarks(benchmarks) => benchmarks_step(payload, benchmarks, today).await?,
            Step::Session(session) => session_step(session).await?,
        });
    }
    Ok(Fields { steps: outcomes })
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error))?;
    let fields = runtime.block_on(scenario(&input.0))?;
    Ok(ProbeOutput::AutoRouterReadApi(Output(Box::new(fields))))
}
