use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

mod fixtures;
mod operations;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Decode {
        now: i64,
        status_code: u16,
        bodies: Vec<String>,
    },
    RecordCounts {
        responses: Vec<fixtures::Object>,
    },
    OutputUriPrediction {
        arns: Vec<String>,
        predictions: Vec<fixtures::Prediction>,
    },
    RetrieveSignedRequest {
        credentials: fixtures::CredentialInput,
        requests: Vec<fixtures::Retrieve>,
    },
    RegionAndTime {
        arns: Vec<String>,
        times: Vec<fixtures::TimeInput>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        operations::run(input)
    }
}
