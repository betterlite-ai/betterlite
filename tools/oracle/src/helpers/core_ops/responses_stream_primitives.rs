mod base;
mod duration;
mod errors;
mod mapped;
mod shapes;
mod support;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    BaseStreamState { rows: Vec<base::ProbeSpec> },
    JsonShapeGuards { payloads: Vec<String> },
    StreamErrorFields { rows: Vec<errors::ErrorRow> },
    MappedStreamFailure { rows: Vec<mapped::MappedRow> },
    DurationBoundary { rows: Vec<duration::DurationRow> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BaseStreamState { rows } => base::answer(rows),
            Input::JsonShapeGuards { payloads } => Ok(shapes::answer(payloads)),
            Input::StreamErrorFields { rows } => Ok(errors::answer(rows)),
            Input::MappedStreamFailure { rows } => mapped::answer(rows),
            Input::DurationBoundary { rows } => duration::answer(rows),
        }
    }
}
