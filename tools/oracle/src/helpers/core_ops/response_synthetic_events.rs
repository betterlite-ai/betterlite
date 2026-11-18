use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

mod probes;
mod support;
mod tests;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Build { rows: Vec<support::Row> },
    Terminal { rows: Vec<support::Row> },
    Stamp { rows: Vec<support::Row> },
    Copy { rows: Vec<support::Row> },
    Dump { rows: Vec<support::Row> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Build { rows } => support::rows_answer(rows, support::build_answer),
            Input::Terminal { rows } => support::rows_answer(rows, support::terminal_answer),
            Input::Stamp { rows } => support::rows_answer(rows, support::stamp_answer),
            Input::Copy { rows } => support::rows_answer(rows, support::copy_answer),
            Input::Dump { rows } => support::rows_answer(rows, support::dump_answer),
        }
    }
}
