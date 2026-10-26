use betterlite_guardrails::lasso::{LassoError, extract_tool_arguments};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input.get("operation").and_then(serde_json::Value::as_str);
        let value = input.get("value").cloned().unwrap_or(serde_json::Value::Null);
        match operation {
            Some("extract_tool_arguments") => Ok(extract_tool_arguments(&value).unwrap_or(serde_json::Value::Null)),
            Some("process_response") => {
                let guardrail = betterlite_guardrails::lasso::LassoGuardrail::new(
                    Some("oracle-key".to_owned()), None, None, None, false,
                ).map_err(|error| helper_error("ConstructorError", error))?;
                guardrail.process_response(value).map(|_| serde_json::json!(null)).map_err(|error| {
                    helper_error(error_class(&error), error)
                })
            }
            _ => Self::not_built("b3778"),
        }
    }
}

fn error_class(error: &LassoError) -> &str {
    match error {
        LassoError::Blocked { .. } => "HTTPException",
        _ => "LassoError",
    }
}

fn helper_error(class: &str, error: LassoError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
