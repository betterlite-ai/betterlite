mod lookup;
mod records;

use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "group", rename_all = "snake_case")]
pub(crate) enum Input {
    PolicyRecordsRecords(records::Call),
    PolicyRecordsLookup(lookup::Call),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PolicyRecordsRecords(call) => records::run(call),
            Input::PolicyRecordsLookup(call) => lookup::run(call),
        }
    }
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "PolicyLookupError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
