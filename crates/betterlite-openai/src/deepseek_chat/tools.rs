use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};

const FUNCTION: &str = "function";
const NAME: &str = "name";
const TOOLS: &str = "tools";
const TOOL_CHOICE: &str = "tool_choice";
const PARALLEL_TOOL_CALLS: &str = "parallel_tool_calls";

pub struct DroppedTools {
    pub params: Object,
    pub dropped_types: Vec<String>,
}

pub fn drop_unsupported_tools(optional_params: &Object) -> DroppedTools {
    let Some(Json::List(tools)) = optional_params.get(TOOLS) else {
        return unchanged(optional_params);
    };
    if tools.iter().all(is_function_tool) {
        return unchanged(optional_params);
    }
    let dropped_types = dropped_types(tools);
    tracing::warn!(
        provider = "deepseek",
        dropped = ?dropped_types,
        "DeepSeek chat completions only supports function tools; dropping unsupported tool types"
    );
    let function_tools: Vec<Json> = tools
        .iter()
        .filter(|tool| is_function_tool(tool))
        .cloned()
        .collect();
    DroppedTools {
        params: cleaned_params(optional_params, function_tools),
        dropped_types,
    }
}

fn unchanged(optional_params: &Object) -> DroppedTools {
    DroppedTools {
        params: optional_params.clone(),
        dropped_types: Vec::new(),
    }
}

fn cleaned_params(optional_params: &Object, function_tools: Vec<Json>) -> Object {
    let mut cleaned = optional_params.clone();
    cleaned.shift_remove(TOOLS);
    if function_tools.is_empty() {
        cleaned.shift_remove(TOOL_CHOICE);
        cleaned.shift_remove(PARALLEL_TOOL_CALLS);
        return cleaned;
    }
    let names = function_tool_names(&function_tools);
    if !tool_choice_matches(cleaned.get(TOOL_CHOICE), &names) {
        cleaned.shift_remove(TOOL_CHOICE);
    }
    cleaned.insert(TOOLS.to_owned(), Json::List(function_tools));
    cleaned
}

fn is_function_tool(tool: &Json) -> bool {
    match tool {
        Json::Object(fields) => fields.get("type").and_then(Json::as_str) == Some(FUNCTION),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

fn function_tool_names(tools: &[Json]) -> BTreeSet<String> {
    tools
        .iter()
        .filter_map(|tool| match tool {
            Json::Object(fields) => fields
                .get(FUNCTION)
                .and_then(Json::as_object)
                .and_then(|function| function.get(NAME))
                .and_then(Json::as_str)
                .map(str::to_owned),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

// A string choice, or none at all, survives; a dict must name a function that survived
fn tool_choice_matches(choice: Option<&Json>, names: &BTreeSet<String>) -> bool {
    let Some(Json::Object(fields)) = choice else {
        return true;
    };
    if fields.get("type").and_then(Json::as_str) != Some(FUNCTION) {
        return false;
    }
    let chosen = fields
        .get(FUNCTION)
        .and_then(Json::as_object)
        .and_then(|function| function.get(NAME))
        .and_then(Json::as_str);
    chosen.is_some_and(|name| names.contains(name))
}

fn dropped_types(tools: &[Json]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for tool in tools {
        if !is_function_tool(tool) {
            names.insert(dropped_type_name(tool));
        }
    }
    names.into_iter().collect()
}

fn dropped_type_name(tool: &Json) -> String {
    if let Json::Object(fields) = tool {
        return match fields.get("type") {
            Some(Json::Text(text)) => text.clone(),
            Some(other) => json_type_name(other).to_owned(),
            None => "none".to_owned(),
        };
    }
    json_type_name(tool).to_owned()
}

const fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::Text(_) => "string",
        Json::List(_) => "array",
        Json::Object(_) => "object",
    }
}
