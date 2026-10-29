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
    let Some(functions) = optional_params.get("functions") else {
        return false;
    };
    functions.is_truthy()
}

#[must_use]
pub fn check_is_function_call(optional_params: Option<&Object>) -> bool {
    let Some(params) = optional_params else {
        return false;
    };
    is_function_call(params)
}

fn copy_grammar_fields(source: &Object, target: &mut Object) {
    for field in CUSTOM_GRAMMAR_FIELDS {
        if let Some(value) = source.get(field) {
            target.insert(field.to_owned(), value.clone());
        }
    }
}

fn flat_to_nested_grammar(format_obj: &Object) -> Object {
    let mut grammar = Object::new();
    copy_grammar_fields(format_obj, &mut grammar);
    [
        ("type".to_owned(), Json::Text("grammar".to_owned())),
        ("grammar".to_owned(), Json::Object(grammar)),
    ]
    .into_iter()
    .collect()
}

fn nested_to_flat_grammar(grammar: &Object) -> Object {
    let mut shaped = Object::new();
    shaped.insert("type".to_owned(), Json::Text("grammar".to_owned()));
    copy_grammar_fields(grammar, &mut shaped);
    shaped
}

fn is_flat_custom_grammar(format_obj: &Object) -> bool {
    let is_grammar = format_obj.get("type").and_then(Json::as_str) == Some("grammar");
    is_grammar && !format_obj.contains_key("grammar")
}

fn nested_custom_grammar(format_obj: &Object) -> Option<&Object> {
    let is_grammar = format_obj.get("type").and_then(Json::as_str) == Some("grammar");
    is_grammar
        .then(|| format_obj.get("grammar").and_then(Json::as_object))
        .flatten()
}

#[must_use]
pub fn custom_tool_to_chat_shape(format_obj: &Object) -> Object {
    if is_flat_custom_grammar(format_obj) {
        flat_to_nested_grammar(format_obj)
    } else {
        format_obj.clone()
    }
}

#[must_use]
pub fn custom_tool_to_responses_shape(format_obj: &Object) -> Object {
    let nested = nested_custom_grammar(format_obj);
    nested.map_or_else(|| format_obj.clone(), nested_to_flat_grammar)
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
