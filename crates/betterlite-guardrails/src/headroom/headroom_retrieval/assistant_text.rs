use crate::primitives::json_value::JsonValue;

use super::response::ResponseView;

// LiteLLM's assistant_text_from_response: chat first, then Anthropic blocks, then Responses message items
pub(crate) fn assistant_text(view: &ResponseView) -> Option<String> {
    chat_text(&view.choices)
        .or_else(|| anthropic_text(&view.content))
        .or_else(|| responses_text(&view.output))
}

fn chat_text(choices: &JsonValue) -> Option<String> {
    let JsonValue::Array(items) = choices else {
        return None;
    };
    let Some(JsonValue::Object(choice)) = items.first() else {
        return None;
    };
    let Some(JsonValue::Object(message)) = choice.get("message") else {
        return None;
    };
    let text = content_to_text(message.get("content"));
    (!text.is_empty()).then_some(text)
}

// content_text::content_to_text reads a GuardrailValue from config; these response fields are primitives::JsonValue
fn content_to_text(content: Option<&JsonValue>) -> String {
    match content {
        Some(JsonValue::String(text)) => text.clone(),
        Some(JsonValue::Array(parts)) => {
            let texts: Vec<&str> = parts.iter().filter_map(text_part).collect();
            texts.join("\n\n")
        }
        _ => String::new(),
    }
}

fn text_part(part: &JsonValue) -> Option<&str> {
    let JsonValue::Object(fields) = part else {
        return None;
    };
    if !matches!(fields.get("type"), Some(JsonValue::String(kind)) if kind == "text") {
        return None;
    }
    match fields.get("text") {
        Some(JsonValue::String(text)) => Some(text.as_str()),
        _ => None,
    }
}

fn anthropic_text(content: &JsonValue) -> Option<String> {
    let JsonValue::Array(blocks) = content else {
        return None;
    };
    let parts: Vec<&str> = blocks
        .iter()
        .filter_map(text_part)
        .filter(|text| !text.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(""))
}

fn responses_text(output: &JsonValue) -> Option<String> {
    let JsonValue::Array(items) = output else {
        return None;
    };
    let parts: Vec<&str> = items.iter().flat_map(message_output_text).collect();
    (!parts.is_empty()).then(|| parts.join(""))
}

fn message_output_text(item: &JsonValue) -> Vec<&str> {
    let JsonValue::Object(fields) = item else {
        return Vec::new();
    };
    if !matches!(fields.get("type"), Some(JsonValue::String(kind)) if kind == "message") {
        return Vec::new();
    }
    let Some(JsonValue::Array(chunks)) = fields.get("content") else {
        return Vec::new();
    };
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            JsonValue::Object(part)
                if matches!(part.get("type"), Some(JsonValue::String(kind)) if kind == "output_text") =>
            {
                match part.get("text") {
                    Some(JsonValue::String(text)) if !text.is_empty() => Some(text.as_str()),
                    _ => None,
                }
            }
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => None,
        })
        .collect()
}
