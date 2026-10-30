use betterlite_error::error::ErrorClass;
use serde_json::to_value;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = ErrorClass;

    fn run(input: &Self::Input) -> HelperResult {
        to_value(input.metadata()).map_err(|error| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
