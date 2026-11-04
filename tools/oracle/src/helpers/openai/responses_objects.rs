use betterlite_openai::api::responses_objects::{
    ResponseApiUsage, ResponsesApiResponse, ResponsesToolUsage,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    UsageCost { usage: ResponseApiUsage },
    ResponseDefaults { response: ResponsesApiResponse },
    ReasoningNulls { response: ResponsesApiResponse },
    OutputTextJoin { response: ResponsesApiResponse },
    ToolUsageCount { tool_usage: ResponsesToolUsage },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::UsageCost { usage } => serde_json::to_value(usage),
            Input::ResponseDefaults { response } => serde_json::to_value(serde_json::json!({
                "response": response,
                "output_text": response.output_text()
            })),
            Input::ReasoningNulls { response } => serde_json::to_value(response),
            Input::OutputTextJoin { response } => serde_json::to_value(response.output_text()),
            Input::ToolUsageCount { tool_usage } => serde_json::to_value(tool_usage),
        };
        output.map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
