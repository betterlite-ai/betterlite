use betterlite_azure::azure_chat_base_policy::SUPPORTED_OPENAI_PARAMS;
use betterlite_azure::azure_chat_base_policy::mapping::requires_max_completion_tokens;
use betterlite_azure::azure_chat_base_policy::messages::sanitized_tools_update;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use input::Input;
use reports::{
    auth_report, class_report, error_report, load_settings, mapping_steps, model_report,
    transform_steps, version_reports,
};

mod input;
mod reports;
mod tests;

const TRANSFORM_MODEL: &str = "gpt-4o";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ApiVersionResponseFormat(call) => {
                Ok(version_reports(call, load_settings(call.settings)?))
            }
            Input::MaxCompletionTokens(call) => Ok(model_report(
                call,
                load_settings(call.settings)?,
                |model| json!({"model": model, "requires": requires_max_completion_tokens(model)}),
            )),
            Input::UnsupportedParam(call) => Ok(model_report(
                call,
                load_settings(call.settings)?,
                |model| json!({"model": model, "params": SUPPORTED_OPENAI_PARAMS}),
            )),
            Input::AzureToolChoiceDateGate(call)
            | Input::AzureRequiredToolChoice(call)
            | Input::AzureParameterShapes(call) => Ok(json!({
                "steps": mapping_steps(&call.steps, load_settings(call.settings)?),
            })),
            Input::AzureMessageContent(call) | Input::AzureToolChoiceWithoutTools(call) => {
                Ok(json!({
                    "steps": transform_steps(&call.steps, load_settings(call.settings)?),
                }))
            }
            Input::SanitizedToolSchema(call) => Ok(json!(sanitized_tools_update(&call.optional))),
            Input::AzureClassDefaults(call) => Ok(json!({
                "steps": call.steps.iter().map(class_report).collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            })),
            Input::AzureSpecialAuthRegions(call) => Ok(auth_report(call)),
            Input::AzureErrorFactory(call) => Ok(json!({
                "steps": call.steps.iter().map(error_report).collect::<Vec<Value>>(),
            })),
        }
    }
}
