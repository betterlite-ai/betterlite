#[path = "../../websocket_driver/responses_ws_managed.rs"]
mod adapter;
use crate::websocket_driver::DriverError;
use crate::websocket_driver::service::{ServiceInput, ServiceTag, Services};
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
        let mut services = Services::default();
        services.register_service(ServiceTag::ManagedResponsesTurns, adapter::service::run)?;
        services.run_service(input.service, &input.input)
    }
}

impl From<DriverError> for Box<HelperError> {
    fn from(error: DriverError) -> Self {
        service_error("ServiceError", error.to_string())
    }
}
