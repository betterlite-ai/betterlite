use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::to_value;

use crate::helpers::{Helper, HelperError, HelperResult};

// exception_type()'s native-family elif chain and its catchall, reached by neither an already-mapped identity nor an openai-family provider
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Raw(Box<RawException>),
    Mapped(ErrorClass),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| {
            Box::new(HelperError {
                class: "RedactionError".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.to_string(),
            })
        })?;
        let dispatch_input = match input {
            Input::Raw(raw) => DispatchInput::Raw(raw.clone()),
            Input::Mapped(class) => DispatchInput::Mapped(class.clone()),
        };
        let class = dispatch(&dispatch_input, &redactor);
        to_value(class.metadata()).map_err(|error| {
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
