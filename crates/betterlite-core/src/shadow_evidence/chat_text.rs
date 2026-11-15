use betterlite_models::decisions::Json;
use betterlite_models::judge_primitives::content_text_extraction::extract_text_from_content;

use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::responses_tool_namespaces::values::has_value;

mod tests;

// LiteLLM reads a mapping by key and a typed object by attribute; a JSON object is the one shape both reach
#[must_use]
pub fn field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub fn final_text(response: &Json) -> String {
    let Some(message) = message(response) else {
        return String::new();
    };
    let prose = match field(message, "content") {
        Some(content) => extract_text_from_content(content),
        None => String::new(),
    };
    let calls = field(message, "tool_calls");
    let single = field(message, "function_call");
    if !(calls.is_some_and(has_value) || single.is_some_and(has_value)) {
        return prose;
    }
    let serialized = serialize_invocations(&listed_calls(message));
    if prose.is_empty() {
        return serialized;
    }
    format!("{prose} {serialized}").trim().to_owned()
}

#[must_use]
pub fn finish_reason(response: &Json) -> String {
    let raw = choice(response).and_then(|choice| field(choice, "finish_reason"));
    match raw.filter(|value| has_value(value)) {
        Some(reason) => string_value(reason),
        None => "unknown".to_owned(),
    }
}

#[must_use]
pub fn empty_reply_error(response: &Json, routed_model: &str) -> String {
    let model = if routed_model.is_empty() {
        "unknown"
    } else {
        routed_model
    };
    format!(
        "shadow router returned an empty response; finish_reason={}, model={model}",
        finish_reason(response)
    )
}

pub(crate) fn message(response: &Json) -> Option<&Json> {
    match choice(response).and_then(|choice| field(choice, "message")) {
        Some(Json::Null) | None => None,
        Some(message) => Some(message),
    }
}

pub(crate) fn serialize_invocations(calls: &[&Json]) -> String {
    let mut serialized: Vec<String> = Vec::with_capacity(calls.len());
    for call in calls {
        serialized.push(format!("[tool call] {}", invocation(call)));
    }
    serialized.join(", ")
}

fn choice(response: &Json) -> Option<&Json> {
    match field(response, "choices") {
        Some(Json::List(choices)) => choices.first(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => None,
    }
}

fn listed_calls(message: &Json) -> Vec<&Json> {
    if let Some(Json::List(calls)) = field(message, "tool_calls")
        && !calls.is_empty()
    {
        return calls.iter().collect();
    }
    match field(message, "function_call") {
        Some(Json::Null) | None => Vec::new(),
        Some(single) => vec![single],
    }
}

fn invocation_payload(call: &Json) -> &Json {
    let function = field(call, "function").filter(|value| has_value(value));
    let custom = field(call, "custom").filter(|value| has_value(value));
    match function.or(custom) {
        Some(payload) => payload,
        None => call,
    }
}

fn invocation(call: &Json) -> String {
    let payload = invocation_payload(call);
    let name = match field(payload, "name").filter(|value| has_value(value)) {
        Some(name) => string_value(name),
        None => "unnamed".to_owned(),
    };
    let arguments = match field(payload, "arguments").filter(|value| has_value(value)) {
        Some(arguments) => string_value(arguments),
        None => match field(payload, "input").filter(|value| has_value(value)) {
            Some(input) => string_value(input),
            None => String::new(),
        },
    };
    format!("{name}({arguments})")
}
