use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::wire::XecGuardMessage;

const DEFAULT_ROLE: &str = "user";

#[must_use]
pub fn normalize_message(message: &GuardrailValue) -> Option<XecGuardMessage> {
    let fields = as_object(message)?;
    let role = match fields.get("role") {
        Some(GuardrailValue::Text(role)) => role.clone(),
        _ => DEFAULT_ROLE.to_owned(),
    };
    let content = fields
        .get("content")
        .and_then(message_content)
        .unwrap_or_default();
    Some(XecGuardMessage { role, content })
}

#[must_use]
pub fn synthesize_user_message(texts: &[String]) -> Option<XecGuardMessage> {
    if texts.is_empty() {
        return None;
    }
    let joined = texts
        .iter()
        .filter(|text| !text.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if joined.is_empty() {
        return None;
    }
    Some(XecGuardMessage {
        role: DEFAULT_ROLE.to_owned(),
        content: joined,
    })
}

#[must_use]
pub fn last_text_by_role(messages: &[XecGuardMessage], role: &str) -> Option<String> {
    let message = messages.iter().rev().find(|message| message.role == role)?;
    if message.content.is_empty() {
        return None;
    }
    Some(message.content.clone())
}

#[must_use]
pub fn assistant_text_from_response(response: &GuardrailValue) -> Option<String> {
    let choices = as_array(as_object(response)?.get("choices")?)?;
    let texts = choices.iter().filter_map(choice_text).collect::<Vec<_>>();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

const fn as_object(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    if let GuardrailValue::Object(fields) = value {
        Some(fields)
    } else {
        None
    }
}

const fn as_array(value: &GuardrailValue) -> Option<&[GuardrailValue]> {
    if let GuardrailValue::Array(items) = value {
        Some(items.as_slice())
    } else {
        None
    }
}

fn message_content(value: &GuardrailValue) -> Option<String> {
    if let GuardrailValue::Text(text) = value {
        return Some(text.clone());
    }
    if let GuardrailValue::Array(parts) = value {
        return Some(
            parts
                .iter()
                .filter_map(text_part)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    None
}

fn choice_text(choice: &GuardrailValue) -> Option<String> {
    let choice_fields = as_object(choice)?;
    let message = as_object(choice_fields.get("message")?)?;
    content_to_text(message.get("content"))
}

fn content_to_text(content: Option<&GuardrailValue>) -> Option<String> {
    if let Some(GuardrailValue::Text(text)) = content {
        if text.is_empty() {
            return None;
        }
        return Some(text.clone());
    }
    if let Some(GuardrailValue::Array(items)) = content {
        let joined = items
            .iter()
            .filter_map(text_part)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if joined.is_empty() {
            return None;
        }
        return Some(joined);
    }
    None
}

fn text_part(item: &GuardrailValue) -> Option<&str> {
    let fields = as_object(item)?;
    let is_text = matches!(
        fields.get("type"),
        Some(GuardrailValue::Text(kind)) if kind == "text"
    );
    if !is_text {
        return None;
    }
    match fields.get("text") {
        Some(GuardrailValue::Text(text)) => Some(text),
        _ => None,
    }
}

mod tests;
