use super::json::{Json, Object};

const CUSTOM_GRAMMAR_FIELDS: [&str; 2] = ["definition", "syntax"];

#[must_use]
pub fn get_tool_call_names(tools: &[Object]) -> Vec<String> {
    let mut names = Vec::new();
    for tool in tools {
        if tool.get("type").and_then(Json::as_str) != Some("function") {
            continue;
        }
        let Some(name) = tool
            .get("function")
            .and_then(Json::as_object)
            .and_then(|function| function.get("name"))
            .and_then(Json::as_str)
        else {
            continue;
        };
        if !name.is_empty() {
            names.push(name.to_owned());
        }
    }
    names
}

#[must_use]
pub fn is_function_call(optional_params: &Object) -> bool {
    optional_params
        .get("functions")
        .is_some_and(Json::is_truthy)
}

#[must_use]
pub fn check_is_function_call(optional_params: Option<&Object>) -> bool {
    optional_params.is_some_and(is_function_call)
}

#[must_use]
pub fn convert_custom_tool_format_to_chat_shape(format_obj: &Object) -> Object {
    let is_grammar = format_obj.get("type").and_then(Json::as_str) == Some("grammar");
    if !is_grammar || format_obj.contains_key("grammar") {
        return format_obj.clone();
    }
    let mut grammar = Object::new();
    for field in CUSTOM_GRAMMAR_FIELDS {
        if let Some(value) = format_obj.get(field) {
            grammar.insert(field.to_owned(), value.clone());
        }
    }
    let mut shaped = Object::new();
    shaped.insert("type".to_owned(), Json::Text("grammar".to_owned()));
    shaped.insert("grammar".to_owned(), Json::Object(grammar));
    shaped
}

#[must_use]
pub fn convert_custom_tool_format_to_responses_shape(format_obj: &Object) -> Object {
    let is_grammar = format_obj.get("type").and_then(Json::as_str) == Some("grammar");
    let Some(grammar) = format_obj.get("grammar").and_then(Json::as_object) else {
        return format_obj.clone();
    };
    if !is_grammar {
        return format_obj.clone();
    }
    let mut shaped = Object::new();
    shaped.insert("type".to_owned(), Json::Text("grammar".to_owned()));
    for field in CUSTOM_GRAMMAR_FIELDS {
        if let Some(value) = grammar.get(field) {
            shaped.insert(field.to_owned(), value.clone());
        }
    }
    shaped
}

#[must_use]
pub fn with_prompt_cache_breakpoint(target: &Object, marker: Option<&Json>) -> Object {
    let Some(breakpoint) = marker else {
        return target.clone();
    };
    let mut result = target.clone();
    result.insert("prompt_cache_breakpoint".to_owned(), breakpoint.clone());
    result
}

mod tests;
