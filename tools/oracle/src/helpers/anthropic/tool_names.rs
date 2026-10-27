use std::collections::BTreeMap;

use betterlite_anthropic::tool_names::wire::{Message, Name, RequestParams};
use betterlite_anthropic::tool_names::{
    ToolName, basic_sanitize_tool_name, build_tool_name_maps, rewrite_tool_names_in_messages,
    sanitize_tool_names_in_request,
};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Basic {
        name: serde_json::Value,
    },
    Map {
        names: Vec<serde_json::Value>,
    },
    Request {
        params: serde_json::Value,
    },
    Messages {
        messages: serde_json::Value,
        forward: BTreeMap<String, String>,
    },
}

pub(crate) struct Op;

fn json_error(error: &serde_json::Error) -> Box<crate::helpers::HelperError> {
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "AnthropicToolNames".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Basic { name } => {
                let encoded = serde_json::to_string(name).map_err(|error| json_error(&error))?;
                let parsed =
                    serde_json::from_str::<Name>(&encoded).map_err(|error| json_error(&error))?;
                serde_json::to_value(serde_json::json!({"name": basic_sanitize_tool_name(&parsed)}))
            }
            Input::Map { names } => {
                let typed_names = names
                    .iter()
                    .filter(|value| value.is_string())
                    .cloned()
                    .map(serde_json::from_value::<ToolName>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| json_error(&error))?;
                serde_json::to_value(build_tool_name_maps(&typed_names))
            }
            Input::Request { params } => {
                let encoded = serde_json::to_string(params).map_err(|error| json_error(&error))?;
                let mut parsed_params = serde_json::from_str::<RequestParams>(&encoded)
                    .map_err(|error| json_error(&error))?;
                let maps = sanitize_tool_names_in_request(&mut parsed_params);
                serde_json::to_value(serde_json::json!({"params": parsed_params, "maps": maps}))
            }
            Input::Messages { messages, forward } => serde_json::to_value(serde_json::json!({
                "messages": rewrite_tool_names_in_messages(
                    &serde_json::from_str::<Vec<Message>>(
                        &serde_json::to_string(messages).map_err(|error| json_error(&error))?,
                    )
                    .map_err(|error| json_error(&error))?,
                    forward,
                )
            })),
        };
        output.map_err(|error| json_error(&error))
    }
}

mod tests;
