use betterlite_error::azure_openrouter::azure::{self, AzureExceptionInput};
use betterlite_error::azure_openrouter::openrouter::{self, OpenrouterExceptionInput};
use serde_json::to_value;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Azure(Box<AzureExceptionInput>),
    Openrouter(OpenrouterExceptionInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let metadata = match input {
            Input::Azure(azure_input) => azure::map_azure_exception(azure_input).metadata(),
            Input::Openrouter(openrouter_input) => {
                openrouter::map_openrouter_exception(openrouter_input).metadata()
            }
        };
        to_value(metadata).map_err(|error| {
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
