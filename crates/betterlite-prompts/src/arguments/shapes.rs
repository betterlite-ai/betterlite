use serde_json::{Map, Value};

const CUSTOM_GRAMMAR_FIELDS: [&str; 2] = ["definition", "syntax"];

pub fn get_tool_call_names(tools: &[Map<String, Value>]) -> Vec<String> {
    let mut names = Vec::new();
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        let Some(name) = tool
            .get("function")
            .and_then(Value::as_object)
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !name.is_empty() {
            names.push(name.to_owned());
        }
    }
    names
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Bool(true) => true,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

pub fn is_function_call(optional_params: &Map<String, Value>) -> bool {
    match optional_params.get("functions") {
        Some(value) => is_truthy(value),
        None => false,
    }
}

pub fn check_is_function_call(optional_params: Option<&Map<String, Value>>) -> bool {
    match optional_params {
        Some(params) => is_function_call(params),
        None => false,
    }
}

pub fn convert_custom_tool_format_to_chat_shape(
    format_obj: &Map<String, Value>,
) -> Map<String, Value> {
    let is_grammar = format_obj.get("type").and_then(Value::as_str) == Some("grammar");
    if !is_grammar || format_obj.contains_key("grammar") {
        return format_obj.clone();
    }
    let mut grammar = Map::new();
    for field in CUSTOM_GRAMMAR_FIELDS {
        if let Some(value) = format_obj.get(field) {
            grammar.insert(field.to_owned(), value.clone());
        }
    }
    let mut shaped = Map::new();
    shaped.insert("type".to_owned(), Value::String("grammar".to_owned()));
    shaped.insert("grammar".to_owned(), Value::Object(grammar));
    shaped
}

pub fn convert_custom_tool_format_to_responses_shape(
    format_obj: &Map<String, Value>,
) -> Map<String, Value> {
    let is_grammar = format_obj.get("type").and_then(Value::as_str) == Some("grammar");
    let Some(grammar) = format_obj.get("grammar").and_then(Value::as_object) else {
        return format_obj.clone();
    };
    if !is_grammar {
        return format_obj.clone();
    }
    let mut shaped = Map::new();
    shaped.insert("type".to_owned(), Value::String("grammar".to_owned()));
    for field in CUSTOM_GRAMMAR_FIELDS {
        if let Some(value) = grammar.get(field) {
            shaped.insert(field.to_owned(), value.clone());
        }
    }
    shaped
}

pub fn with_prompt_cache_breakpoint(
    target: &Map<String, Value>,
    marker: Option<&Value>,
) -> Map<String, Value> {
    let Some(marker) = marker else {
        return target.clone();
    };
    let mut marked = target.clone();
    marked.insert("prompt_cache_breakpoint".to_owned(), marker.clone());
    marked
}

mod tests;
