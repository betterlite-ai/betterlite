use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::Json;

// LiteLLM reads input only from a typed params object; a plain mapping carries nothing the config looks at
#[derive(Clone, Copy, Debug)]
pub enum LiteLlmParams<'input> {
    Typed {
        input: Option<&'input ResponsesInput>,
    },
    Map,
}

// a missing or falsey role marks agent traffic, like an assistant role; a tool role alone stays a user turn
#[must_use]
pub fn initiator(input: &ResponsesInput) -> &'static str {
    let ResponsesInput::Items(items) = input else {
        return "user";
    };
    for item in items {
        let Json::Object(fields) = &item.0 else {
            continue;
        };
        let agent = match fields.get("role") {
            None | Some(Json::Null | Json::Bool(false)) => true,
            Some(Json::Text(role)) => role.is_empty() || role.eq_ignore_ascii_case("assistant"),
            Some(Json::Number(number)) => number.as_i64() == Some(0),
            Some(Json::List(parts)) => parts.is_empty(),
            Some(Json::Object(entries)) => entries.is_empty(),
            Some(Json::Bool(true)) => false,
        };
        if agent {
            return "agent";
        }
    }
    "user"
}

// the walk follows lists and content lists and stops, with a warning, only beyond max_depth
#[must_use]
pub fn has_vision_input(input: &ResponsesInput, max_depth: i64) -> bool {
    if beyond_depth(0, max_depth) {
        return false;
    }
    let ResponsesInput::Items(items) = input else {
        return false;
    };
    items
        .iter()
        .any(|item| contains_vision(&item.0, 1, max_depth))
}

fn beyond_depth(depth: i64, max_depth: i64) -> bool {
    let beyond = depth > max_depth;
    if beyond {
        tracing::warn!(
            max_depth,
            "[GitHub Copilot] Max recursion depth reached while checking for vision content"
        );
    }
    beyond
}

fn contains_vision(value: &Json, depth: i64, max_depth: i64) -> bool {
    if beyond_depth(depth, max_depth) {
        return false;
    }
    let parts = match value {
        Json::List(items) => items,
        Json::Object(fields) => {
            if matches!(fields.get("type"), Some(Json::Text(kind)) if kind.eq_ignore_ascii_case("input_image"))
            {
                return true;
            }
            let Some(Json::List(content)) = fields.get("content") else {
                return false;
            };
            content
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return false,
    };
    parts
        .iter()
        .any(|part| contains_vision(part, depth.saturating_add(1), max_depth))
}

// the Copilot override drops every null field of a reasoning item, encrypted_content included, and leaves other items alone
#[must_use]
pub fn validated_input(input: &ResponsesInput) -> ResponsesInput {
    let ResponsesInput::Items(items) = input else {
        return input.clone();
    };
    let validated = items
        .iter()
        .map(|item| match &item.0 {
            Json::Object(fields)
                if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "reasoning") =>
            {
                ResponsesItem(Json::Object(
                    fields
                        .iter()
                        .filter(|(_name, value)| !matches!(value, Json::Null))
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                ))
            }
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_) => item.clone(),
        })
        .collect();
    ResponsesInput::Items(validated)
}
