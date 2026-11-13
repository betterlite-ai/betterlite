use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;

use crate::classifier_message_text::asks::{json_content, json_role, role_name};
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::MarkerPair;
use crate::classifier_message_text::text::{
    human_text, json_human_text, json_message_text, message_text,
};

pub const PLAN_MODE_TOOL_NAME: &str = "exit_plan_mode";

pub const PLAN_MODE_SYSTEM_SENTINELS: &[&str] = &["You are currently running in \"Plan\" mode."];

pub const PLAN_MODE_TAIL_SENTINELS: &[&str] = &["Plan mode is active", "Plan mode still active"];

enum PlanMessage<'message> {
    Raw(&'message Json),
    Typed(&'message ChatMessage),
}

impl PlanMessage<'_> {
    fn role(&self) -> Option<&str> {
        match self {
            Self::Raw(message) => json_role(message),
            Self::Typed(message) => Some(role_name(message.role)),
        }
    }

    fn text(&self) -> Result<String, ClassifierTextError> {
        match self {
            Self::Raw(message) => json_message_text(json_content(message)),
            Self::Typed(message) => message_text(message.content.as_ref()),
        }
    }

    fn human_text(&self, markers: &[MarkerPair<'_>]) -> Result<String, ClassifierTextError> {
        match self {
            Self::Raw(message) => json_human_text(json_content(message), markers),
            Self::Typed(message) => human_text(message.content.as_ref(), markers),
        }
    }
}

#[must_use]
pub fn matched_plan_mode_sentinel(
    body: Option<&Json>,
    resolved_messages: Option<&[ChatMessage]>,
    extra_patterns: &[String],
    markers: &[MarkerPair<'_>],
) -> Result<Option<String>, ClassifierTextError> {
    let tools = body.and_then(|value| object_field(value, "tools"));
    if has_tool_with_name(tools, PLAN_MODE_TOOL_NAME) {
        return Ok(Some(PLAN_MODE_TOOL_NAME.to_owned()));
    }
    let body_messages = body.and_then(|value| object_field(value, "messages"));
    let messages: Vec<PlanMessage<'_>> = match body_messages {
        Some(Json::List(items)) => items
            .iter()
            .filter(|item| matches!(item, Json::Object(_)))
            .map(PlanMessage::Raw)
            .collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => resolved_messages
            .map(|messages| messages.iter().map(PlanMessage::Typed).collect())
            .unwrap_or_default(),
    };
    let patterns: Vec<&str> = PLAN_MODE_SYSTEM_SENTINELS
        .iter()
        .chain(PLAN_MODE_TAIL_SENTINELS)
        .copied()
        .chain(extra_patterns.iter().map(String::as_str))
        .collect();
    let system = body.and_then(|value| object_field(value, "system"));
    for text in system_scope_texts(system, &messages)? {
        if let Some(pattern) = contained(&patterns, &text) {
            return Ok(Some(pattern.to_owned()));
        }
    }
    let tail_start = plan_last_human_ask_index(&messages, markers)?.unwrap_or(0);
    for message in messages.iter().skip(tail_start) {
        let text = message.text()?;
        if let Some(pattern) = contained(&patterns, &text) {
            return Ok(Some(pattern.to_owned()));
        }
    }
    Ok(None)
}

// the body's leading system text is rebuilt every request, so a sentinel there is current by construction
fn system_scope_texts(
    body_system: Option<&Json>,
    messages: &[PlanMessage<'_>],
) -> Result<Vec<String>, ClassifierTextError> {
    let mut texts: Vec<String> = Vec::new();
    match body_system {
        Some(Json::Text(text)) => texts.push(text.clone()),
        Some(Json::List(_)) => texts.push(json_message_text(body_system)?),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => {}
    }
    for message in messages {
        if message.role() != Some("system") {
            break;
        }
        let text = message.text()?;
        if !text.is_empty() {
            texts.push(text);
        }
    }
    Ok(texts)
}

fn plan_last_human_ask_index(
    messages: &[PlanMessage<'_>],
    markers: &[MarkerPair<'_>],
) -> Result<Option<usize>, ClassifierTextError> {
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role() != Some("user") {
            continue;
        }
        if !message.human_text(markers)?.is_empty() {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

#[must_use]
pub fn has_tool_with_name(tools: Option<&Json>, name: &str) -> bool {
    let Some(Json::List(items)) = tools else {
        return false;
    };
    items.iter().any(|tool| tool_has_name(tool, name))
}

// LiteLLM matches an OpenAI-shaped tool on its function name and every other shape on its top-level name
fn tool_has_name(tool: &Json, name: &str) -> bool {
    let Json::Object(fields) = tool else {
        return false;
    };
    let function = fields.get("function");
    let function_shaped = matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "function")
        && matches!(function, Some(Json::Object(_)));
    if function_shaped {
        return matches!(
            function,
            Some(Json::Object(object))
                if matches!(object.get("name"), Some(Json::Text(found)) if found == name)
        );
    }
    matches!(fields.get("name"), Some(Json::Text(found)) if found == name)
}

fn contained<'pattern>(patterns: &[&'pattern str], text: &str) -> Option<&'pattern str> {
    patterns
        .iter()
        .copied()
        .find(|pattern| text.contains(*pattern))
}

fn object_field<'value>(value: &'value Json, key: &str) -> Option<&'value Json> {
    let Json::Object(fields) = value else {
        return None;
    };
    fields.get(key)
}
