use serde_json::json;
use tokio::runtime::Builder;

use crate::helpers::{Helper, HelperResult};

use input::Input;
use reports::{
    base_rows, capability_report, invalid_input, reply_rows, resource_rows, transform_rows,
    websocket_report,
};

mod fixture;
mod input;
mod reports;
mod sources;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| invalid_input(error.to_string()))?;
        match input {
            Input::BaseAndApiVersion(call) => Ok(json!({"steps": base_rows(call)})),
            Input::StripModelPrefix(call) | Input::ReasoningNullFields(call) => {
                Ok(json!({"steps": transform_rows(call)?}))
            }
            Input::WebsocketModelQuery(call) => Ok(websocket_report(call)),
            Input::ResourceIdBeforeQuery(call) => Ok(json!({"steps": resource_rows(call)?})),
            Input::CancelResponse(call) => Ok(json!({"steps": reply_rows(call)})),
            Input::OSeriesOptions(call) | Input::AzureResponsesCapabilities(call) => {
                Ok(capability_report(&runtime, call))
            }
        }
    }
}
