use crate::helpers::{Helper, HelperError, HelperResult};
use crate::router_probes::routing_read_batch::Input;
use crate::router_probes::routing_read_batch::scenario;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = scenario::run(input).map_err(|error| {
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
