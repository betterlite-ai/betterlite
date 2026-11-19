use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

mod runs;
mod tests;

use betterlite_oracle_helper_api::error::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "case", rename_all = "snake_case")]
pub(crate) enum Input {
    Responses { runs: Vec<runs::ResponsesRun> },
    Passthrough { runs: Vec<runs::PassthroughRun> },
    Search { runs: Vec<runs::SearchRun> },
    VectorRetrieve { runs: Vec<runs::VectorRun> },
    RealtimeBridge { runs: Vec<runs::RealtimeRun> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let rows = match input {
            Input::Responses { runs } => runs
                .iter()
                .map(runs::responses_run::responses_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::Passthrough { runs } => runs
                .iter()
                .map(runs::passthrough_run::passthrough_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::Search { runs } => runs
                .iter()
                .map(runs::search_run::search_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::VectorRetrieve { runs } => runs
                .iter()
                .map(runs::vector_run::vector_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::RealtimeBridge { runs } => runs
                .iter()
                .map(runs::realtime_run::realtime_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
        };
        Ok(json!({ "runs": rows }))
    }
}
