use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

mod runs;
mod tests;

use betterlite_oracle_helper_api::error::HelperError;
use runs::passthrough_run::passthrough_row;
use runs::realtime_run::realtime_row;
use runs::responses_run::responses_row;
use runs::search_run::search_row;
use runs::vector_run::vector_row;
type Rows = Result<Vec<Value>, Box<HelperError>>;

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
            Input::Responses { runs } => runs.iter().map(responses_row).collect::<Rows>(),
            Input::Passthrough { runs } => runs.iter().map(passthrough_row).collect::<Rows>(),
            Input::Search { runs } => runs.iter().map(search_row).collect::<Rows>(),
            Input::VectorRetrieve { runs } => runs.iter().map(vector_row).collect::<Rows>(),
            Input::RealtimeBridge { runs } => runs.iter().map(realtime_row).collect::<Rows>(),
        }?;
        Ok(json!({ "runs": rows }))
    }
}
