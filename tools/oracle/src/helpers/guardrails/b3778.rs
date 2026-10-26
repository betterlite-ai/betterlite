use betterlite_guardrails::lasso::{
    LassoError, LassoGuardrail, LassoSettings, extract_tool_arguments,
    wire::{LassoResponse, OpenAiTool},
};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input.get("operation").and_then(serde_json::Value::as_str);
        let value = input
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        match operation {
            Some("extract_tool_arguments") => {
                let tool: OpenAiTool = serde_json::from_value(value)
                    .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
                serde_json::to_value(extract_tool_arguments(&tool))
                    .map_err(|error| helper_error("SerializationError", error.to_string()))
            }
            Some("process_response") => {
                let guardrail = LassoGuardrail::new(
                    LassoSettings {
                        api_key: Some("oracle-key".to_owned()),
                        api_base: None,
                        user_id: None,
                        conversation_id: None,
                        mask: false,
                    },
                    reqwest::Client::default(),
                )
                .map_err(|error| helper_error("ConstructorError", error.to_string()))?;
                let response: LassoResponse = serde_json::from_value(value)
                    .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
                guardrail
                    .process_response(response)
                    .map(|()| serde_json::Value::Null)
                    .map_err(|error| helper_error(error_class(&error), error.to_string()))
            }
            _ => Self::not_built("b3778"),
        }
    }
}

const fn error_class(error: &LassoError) -> &str {
    match error {
        LassoError::Blocked { .. } => "HTTPException",
        LassoError::MissingSecrets
        | LassoError::Cache { .. }
        | LassoError::Request { .. }
        | LassoError::HttpStatus { .. }
        | LassoError::ResponseJson { .. } => "LassoError",
    }
}

fn helper_error(class_name: &str, message: String) -> Box<crate::helpers::HelperError> {
    let class = class_name.to_owned();
    Box::new(crate::helpers::HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
