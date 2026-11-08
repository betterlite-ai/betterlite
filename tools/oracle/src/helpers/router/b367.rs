use crate::helpers::{Helper, HelperError, HelperResult};
use crate::router_probes::wildcard_catalog::{Input, execute};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = execute(input).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidRequest".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        serde_json::to_value(output).map_err(|error| {
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
