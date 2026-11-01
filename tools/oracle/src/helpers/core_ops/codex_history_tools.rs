use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Normalize {
        #[serde(rename = "input")]
        items: Value,
    },
    DropUnsupportedTools {
        #[serde(default)]
        tools: Vec<Value>,
        #[serde(default)]
        supported_types: BTreeSet<String>,
    },
    HoistAdditionalTools {
        #[serde(rename = "input")]
        items: Value,
        #[serde(default)]
        tools: Option<Vec<Value>>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Normalize { items } => normalize(items),
            Input::DropUnsupportedTools {
                tools,
                supported_types,
            } => drop_unsupported(tools, supported_types),
            Input::HoistAdditionalTools { items, tools } => hoist(items, tools.as_deref()),
        };
        Ok(output)
    }
}

fn normalize(value: &Value) -> Value {
    let (normalized, rewritten_types) =
        betterlite_core::api::codex_history_tools::normalize_codex_input_items(value);
    let parsed_input = parse_tool_arguments(normalized);
    serde_json::json!({"input": parsed_input, "rewritten_types": rewritten_types})
}

fn parse_tool_arguments(value: Value) -> Value {
    let Value::Array(items) = value else {
        return value;
    };
    let parsed_items = items.into_iter().map(parse_tool_argument_item).collect();
    Value::Array(parsed_items)
}

fn parse_tool_argument_item(item: Value) -> Value {
    let Value::Object(mut object) = item else {
        return item;
    };
    let is_function_call = object.get("type").and_then(Value::as_str) == Some("function_call");
    if is_function_call
        && let Some(Value::String(arguments)) = object.get("arguments")
        && let Ok(parsed) = serde_json::from_str::<Value>(arguments)
    {
        object.insert("arguments".to_owned(), parsed);
    }
    Value::Object(object)
}

fn drop_unsupported(tools: &[Value], supported_types: &BTreeSet<String>) -> Value {
    let (kept, dropped_types) =
        betterlite_core::api::codex_history_tools::drop_unsupported_tools(tools, supported_types);
    serde_json::json!({"tools": kept, "dropped_types": dropped_types})
}

fn hoist(value: &Value, tools: Option<&[Value]>) -> Value {
    let result = betterlite_core::api::codex_history_tools::hoist_additional_tools(value, tools);
    serde_json::json!({"input": result.input, "tools": result.tools, "hoisted": result.hoisted})
}
