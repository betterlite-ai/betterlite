use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Filter { schema: Value },
    Output { schema: Option<Value> },
    Tool { schema: Option<Value> },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Filter { schema } => serde_json::to_value(
                betterlite_anthropic::output_schema::filter_anthropic_output_schema(schema.clone()),
            )
            .map_err(error),
            Input::Output {
                schema: Some(schema),
            } => serde_json::to_value(betterlite_anthropic::output_schema::map_output(Some(
                betterlite_anthropic::output_schema::ResponseFormat::JsonSchema {
                    schema: schema.clone(),
                },
            )))
            .map_err(error),
            Input::Output { schema: None } => Ok(Value::Null),
            Input::Tool { schema } => serde_json::to_value(
                betterlite_anthropic::output_schema::create_json_tool_call(schema.clone()),
            )
            .map_err(error),
        }
    }
}

fn error(error: serde_json::Error) -> Box<crate::helpers::HelperError> {
    let helper_error = crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    };
    Box::new(helper_error)
}

mod tests;
