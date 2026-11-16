use crate::websocket_driver::service::{ServiceInput, Services};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

fn service_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl Helper for Op {
    type Input = ServiceInput;

    fn run(input: &Self::Input) -> HelperResult {
        let output = Services::default()
            .run_service(input.service, &input.input)
            .map_err(|error| service_error("ServiceError", error.to_string()))?;
        serde_json::to_value(output)
            .map_err(|error| service_error("SerializationError", error.to_string()))
    }
}
