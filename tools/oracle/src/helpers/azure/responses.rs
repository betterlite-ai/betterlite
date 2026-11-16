use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use input::Input;
use reports::{
    base_rows, capability_report, projected_body, reply_row, resource_row, websocket_report,
};
use serde_json::json;
use tokio::runtime::Builder;
mod fixture;
mod input;
mod reports;
mod tests;
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        match input {
            Input::BaseAndApiVersion(call) => Ok(json!({"steps": base_rows(call)})),
            Input::StripModelPrefix(call) | Input::ReasoningNullFields(call) => {
                let steps = call
                    .steps
                    .iter()
                    .map(projected_body)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"steps": steps}))
            }
            Input::WebsocketModelQuery(call) => Ok(websocket_report(call)),
            Input::ResourceIdBeforeQuery(call) => {
                Ok(json!({"steps": call.steps.iter().map(resource_row).collect::<Vec<_>>()}))
            }
            Input::CancelResponse(call) => {
                Ok(json!({"steps": call.steps.iter().map(reply_row).collect::<Vec<_>>()}))
            }
            Input::OSeriesOptions(call) | Input::AzureResponsesCapabilities(call) => {
                Ok(capability_report(&runtime, call))
            }
        }
    }
}
