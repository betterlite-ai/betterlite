use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Filter { schema: Json },
    Output { schema: Option<Json> },
    Tool { schema: Option<Json> },
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
            Input::Output { schema: None } => Ok(serde_json::from_str("null").unwrap()),
            Input::Tool { schema } => serde_json::to_value(
                betterlite_anthropic::output_schema::json_tool(schema.clone()),
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
