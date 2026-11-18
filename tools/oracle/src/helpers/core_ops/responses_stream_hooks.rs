use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

mod fixtures;
mod runner;
mod tests;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Cache,
    Logging,
    Headers,
    Failed,
    Chunk,
    Metadata,
    Failure,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    rows: Vec<fixtures::Row>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let rows = input
            .rows
            .iter()
            .map(|row| runtime.block_on(runner::run(input.operation, row)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "rows": rows }))
    }
}
