use betterlite_a2a::send_params::build_message_send_params;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    params: serde_json::Value,
}

fn serialization_error(error: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match build_message_send_params(&input.params) {
            Ok(params) => serde_json::to_value(params).map_err(|error| serialization_error(&error)),
            Err(error) => Err(Box::new(HelperError {
                class: "SendParamsError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })),
        }
    }
}
