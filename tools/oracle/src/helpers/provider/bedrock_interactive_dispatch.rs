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
            Input::Responses { runs } => rows(runs, runs::responses_run::responses_row)?,
            Input::Passthrough { runs } => rows(runs, runs::passthrough_run::passthrough_row)?,
            Input::Search { runs } => rows(runs, runs::search_run::search_row)?,
            Input::VectorRetrieve { runs } => rows(runs, runs::vector_run::vector_row)?,
            Input::RealtimeBridge { runs } => rows(runs, runs::realtime_run::realtime_row)?,
        };
        Ok(json!({ "runs": rows }))
    }
}

fn rows<T>(runs: &[T], row: fn(&T) -> HelperResult) -> Result<Vec<Value>, Box<HelperError>> {
    runs.iter().map(row).collect()
}
