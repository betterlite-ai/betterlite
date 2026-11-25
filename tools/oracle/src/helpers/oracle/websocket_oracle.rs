#[path = "../../websocket_driver/responses_ws_managed.rs"]
mod adapter;
use self::adapter::{ManagedInput, managed};
use crate::websocket_driver::service::{ServiceInput, Services};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    Managed(Box<ManagedInput>),
    Existing(ServiceInput),
}

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
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let selected = match input {
            Input::Managed(envelope) => {
                let ManagedInput::ManagedResponsesTurns(turns) = envelope.as_ref();
                return managed::Op::run(turns);
            }
            Input::Existing(existing) => existing,
        };
        let output = Services::default()
            .run_service(selected.service, &selected.input)
            .map_err(|error| service_error("ServiceError", error.to_string()))?;
        serde_json::to_value(output)
            .map_err(|error| service_error("SerializationError", error.to_string()))
    }
}
