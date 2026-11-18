use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use tokio::runtime::Builder;

mod probe;
mod runner;
mod support;
mod tests;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransformMode {
    Configured,
    Absent,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EstimateMode {
    Fixed,
    Absent,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CostMode {
    Positive,
    Missing,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    payloads: Vec<String>,
    model: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    request_data: IndexMap<String, Json>,
    #[serde(default)]
    metadata: IndexMap<String, Json>,
    transform: TransformMode,
    estimate: EstimateMode,
    cost: CostMode,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(runner::run(input))
    }
}
