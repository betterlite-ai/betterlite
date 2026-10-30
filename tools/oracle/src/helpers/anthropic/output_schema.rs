use betterlite_anthropic::output_schema::{SchemaError, filter_anthropic_output_schema, mapping};
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Filter {
        schema: Json,
    },
    Output {
        schema: Option<Json>,
    },
    Tool {
        schema: Option<Json>,
    },
    MapOutput {
        value: Option<mapping::ResponseFormat>,
    },
    MapTool {
        value: Option<mapping::ResponseFormat>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::Filter { schema } => filter_anthropic_output_schema(schema.clone())
                .and_then(|result| serde_json::to_value(result).map_err(SchemaError::Encode)),
            Input::Output { schema } => {
                let format = mapping::ResponseFormat {
                    type_name: Some(String::from("json_schema")),
                    response_schema: schema
                        .clone()
                        .map_or(mapping::SchemaField::Missing, mapping::SchemaField::Value),
                    json_schema: None,
                };
                mapping::map_output(Some(&format))
                    .and_then(|result| serde_json::to_value(result).map_err(SchemaError::Encode))
            }
            Input::Tool { schema } => mapping::json_tool(schema.as_ref())
                .and_then(|result| serde_json::to_value(result).map_err(SchemaError::Encode)),
            Input::MapOutput { value } => mapping::map_output(value.as_ref())
                .and_then(|result| serde_json::to_value(result).map_err(SchemaError::Encode)),
            Input::MapTool { value } => mapping::map_tool(value.as_ref())
                .and_then(|result| serde_json::to_value(result).map_err(SchemaError::Encode)),
        };
        result.map_err(|source| {
            Box::new(crate::helpers::HelperError {
                class: match source {
                    SchemaError::MissingType => "KeyError",
                    SchemaError::Invalid(_) => "TypeError",
                    SchemaError::Limit
                    | SchemaError::Definitions(_)
                    | SchemaError::Encode(_)
                    | SchemaError::Format(_) => "ValueError",
                }
                .to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

mod tests;
