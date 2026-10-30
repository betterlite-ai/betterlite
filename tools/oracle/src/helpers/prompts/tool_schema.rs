use crate::helpers::{Helper, HelperResult};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::schema::{
    drop_non_python_regex_patterns as drop_regex, flatten_and_drop,
    flatten_top_level as flatten_top, tool_with_sanitized_parameters as sanitize_tool,
};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    FlattenTopLevel {
        schema: Object,
    },
    DropNonPythonRegexPatterns {
        schema: Object,
    },
    #[serde(rename = "flatten_combinators_and_drop_non_python_regex_patterns")]
    FlattenAndDrop {
        schema: Object,
    },
    ToolWithSanitizedParameters {
        tool: Object,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::FlattenTopLevel { schema } => flatten_top(schema),
            Input::DropNonPythonRegexPatterns { schema } => drop_regex(schema),
            Input::FlattenAndDrop { schema } => flatten_and_drop(schema),
            Input::ToolWithSanitizedParameters { tool } => {
                sanitize_tool(tool.clone(), flatten_and_drop)
            }
        };
        Ok(object_to_value(output))
    }
}

fn json_to_value(json: Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(flag),
        Json::Number(number) => Value::Number(number),
        Json::Text(text) => Value::String(text),
        Json::List(items) => Value::Array(items.into_iter().map(json_to_value).collect()),
        Json::Object(fields) => object_to_value(fields),
    }
}

fn object_to_value(object: Object) -> Value {
    let entries: Map<String, Value> = object
        .into_iter()
        .map(|(key, value)| (key, json_to_value(value)))
        .collect();
    Value::Object(entries)
}

mod tests;
