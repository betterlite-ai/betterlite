#[path = "../core_ops/responses_ws_managed/fixtures/service.rs"]
mod service;
use crate::websocket_driver::DriverError;
use crate::websocket_driver::service::{ServiceInput, ServiceTag, Services};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = ServiceInput;

    fn run(input: &Self::Input) -> HelperResult {
        let mut services = Services::default();
        services.register_service(ServiceTag::ManagedResponsesTurns, service::run)?;
        services.run_service(input.service, &input.input)
    }
}

impl From<DriverError> for Box<HelperError> {
    fn from(error: DriverError) -> Self {
        Self::new(HelperError {
            class: String::from("ServiceError"),
            message: error.to_string(),
            status: None,
            error_type: None,
            param: None,
            code: None,
        })
    }
}
