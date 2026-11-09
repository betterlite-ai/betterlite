use betterlite_mcp::sampling_convert::messages::convert_messages;
use betterlite_mcp::sampling_convert::model_preferences::resolve_model_from_preferences;
use betterlite_mcp::sampling_convert::response::convert_response;
use betterlite_mcp::sampling_convert::tools::{convert_tool_choice, convert_tools};
use serde::Serialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::input::{Input, MessageScenario, ResolveScenario, ResponseScenario, ToolScenario};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Resolve(resolve) => Ok(json!({
            "results": resolve.scenarios.iter().map(resolve_value).collect::<Vec<_>>(),
        })),
        Input::Messages(messages) => {
            let results = messages
                .scenarios
                .iter()
                .map(messages_value)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({ "results": results }))
        }
        Input::Tools(tools) => Ok(json!({
            "results": tools.scenarios.iter().map(tools_value).collect::<Vec<_>>(),
        })),
        Input::Response(response) => Ok(json!({
            "results": response.scenarios.iter().map(response_value).collect::<Vec<_>>(),
        })),
    }
}

fn resolve_value(scenario: &ResolveScenario) -> Value {
    let resolved = resolve_model_from_preferences(
        scenario.preferences.as_ref(),
        &scenario.available_models,
        scenario.default_model.as_deref(),
        scenario.fallback_model.as_deref(),
        &scenario.model_info,
    );
    match resolved {
        Ok(model) => json!({ "outcome": "resolved", "model": model }),
        Err(_error) => json!({ "outcome": "failed" }),
    }
}

fn messages_value(scenario: &MessageScenario) -> Result<Value, Box<HelperError>> {
    let converted = convert_messages(&scenario.messages, scenario.system_prompt.as_deref());
    let mut value = value_of(&converted)?;
    normalise_arguments(&mut value);
    Ok(value)
}

fn tools_value(scenario: &ToolScenario) -> Value {
    let tools = convert_tools(scenario.tools.as_deref());
    let tool_choice = convert_tool_choice(scenario.tool_choice.as_ref());
    json!({ "tools": tools, "tool_choice": tool_choice })
}

fn response_value(scenario: &ResponseScenario) -> Value {
    match convert_response(&scenario.response, &scenario.model_name) {
        Ok(result) => json!({ "outcome": "result", "value": result }),
        Err(_error) => json!({ "outcome": "failed" }),
    }
}

// both sides compare the parsed arguments object, since Python writes json.dumps spacing we do not copy
fn normalise_arguments(value: &mut Value) {
    let Some(messages) = value.as_array_mut() else {
        return;
    };
    for message in messages {
        let Some(tool_calls) = message.get_mut("tool_calls").and_then(Value::as_array_mut) else {
            continue;
        };
        for call in tool_calls {
            let Some(arguments) = call
                .get_mut("function")
                .and_then(|function| function.get_mut("arguments"))
            else {
                continue;
            };
            let Some(text) = arguments.as_str() else {
                continue;
            };
            let Ok(parsed) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            *arguments = parsed;
        }
    }
}

fn value_of<T>(value: &T) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: String::from("SerializationError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}
