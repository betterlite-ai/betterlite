use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};

use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::MarkerPair;
use crate::classifier_message_text::text::{
    human_text, json_human_text, message_text, strip_reminder_blocks,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedAsk {
    pub text: String,
    pub message_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPrompt {
    pub text: String,
    pub message_index: usize,
}

#[must_use]
pub fn human_asks_newest_first(
    messages: &[ChatMessage],
    markers: &[MarkerPair<'_>],
) -> Result<Vec<ExtractedAsk>, ClassifierTextError> {
    let mut asks: Vec<ExtractedAsk> = Vec::new();
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role != Role::User {
            continue;
        }
        let text = human_text(message.content.as_ref(), markers)?;
        if !text.is_empty() {
            asks.push(ExtractedAsk {
                text,
                message_index: index,
            });
        }
    }
    Ok(asks)
}

#[must_use]
pub fn conversation_is_continuing(messages: Option<&[ChatMessage]>) -> bool {
    match messages {
        None => true,
        Some(messages) => {
            messages.is_empty()
                || messages
                    .iter()
                    .any(|message| message.role == Role::Assistant)
        }
    }
}

#[must_use]
pub fn newest_turn_ask(
    messages: &[ChatMessage],
    markers: &[MarkerPair<'_>],
) -> Result<Option<ExtractedAsk>, ClassifierTextError> {
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role != Role::User {
            continue;
        }
        let text = human_text(message.content.as_ref(), markers)?;
        return Ok((!text.is_empty()).then_some(ExtractedAsk {
            text,
            message_index: index,
        }));
    }
    Ok(None)
}

#[must_use]
pub fn current_ask_and_system_prompt(
    messages: &[ChatMessage],
    markers: &[MarkerPair<'_>],
) -> Result<(Option<ExtractedAsk>, Option<SystemPrompt>), ClassifierTextError> {
    let current_ask = human_asks_newest_first(messages, markers)?
        .into_iter()
        .next();
    let mut system_prompt = None;
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role != Role::System {
            continue;
        }
        let text = message_text(message.content.as_ref())?;
        if !text.is_empty() {
            system_prompt = Some(SystemPrompt {
                text,
                message_index: index,
            });
            break;
        }
    }
    Ok((current_ask, system_prompt))
}

#[must_use]
pub fn last_human_ask_index(
    messages: &[ChatMessage],
    markers: &[MarkerPair<'_>],
) -> Result<Option<usize>, ClassifierTextError> {
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role != Role::User {
            continue;
        }
        if !human_text(message.content.as_ref(), markers)?.is_empty() {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

#[must_use]
pub fn is_reminder_only_turn(
    message: &ChatMessage,
    markers: &[MarkerPair<'_>],
) -> Result<bool, ClassifierTextError> {
    if message.role != Role::User {
        return Ok(false);
    }
    let text_only = match message.content.as_ref() {
        None => false,
        Some(MessageContent::Text(_)) => true,
        Some(MessageContent::Parts(parts)) => parts.iter().all(|part| part.kind == "text"),
    };
    if !text_only {
        return Ok(false);
    }
    let text = message_text(message.content.as_ref())?;
    Ok(!text.trim().is_empty() && strip_reminder_blocks(&text, markers).is_empty())
}

#[must_use]
pub fn newest_turn_is_human_ask(
    messages: Option<&[ChatMessage]>,
    markers: &[MarkerPair<'_>],
) -> Result<bool, ClassifierTextError> {
    let Some(messages) = messages.filter(|messages| !messages.is_empty()) else {
        return Ok(false);
    };
    let mut newest_activity = None;
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role != Role::System && !is_reminder_only_turn(message, markers)? {
            newest_activity = Some(index);
            break;
        }
    }
    let Some(newest_activity) = newest_activity else {
        return Ok(false);
    };
    Ok(last_human_ask_index(messages, markers)? == Some(newest_activity))
}

#[must_use]
pub fn json_human_asks_newest_first(
    messages: &[Json],
    markers: &[MarkerPair<'_>],
) -> Result<Vec<ExtractedAsk>, ClassifierTextError> {
    let mut asks: Vec<ExtractedAsk> = Vec::new();
    for (index, message) in messages.iter().enumerate().rev() {
        if json_role(message) != Some("user") {
            continue;
        }
        let text = json_human_text(json_content(message), markers)?;
        if !text.is_empty() {
            asks.push(ExtractedAsk {
                text,
                message_index: index,
            });
        }
    }
    Ok(asks)
}

#[must_use]
pub fn json_role(message: &Json) -> Option<&str> {
    let Json::Object(fields) = message else {
        return None;
    };
    match fields.get("role") {
        Some(Json::Text(role)) => Some(role.as_str()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

#[must_use]
pub fn json_content(message: &Json) -> Option<&Json> {
    let Json::Object(fields) = message else {
        return None;
    };
    fields.get("content")
}

// one role name for a typed message and a raw one, so plan-mode scans a body's messages and typed ones alike
#[must_use]
pub const fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::Developer => "developer",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::Function => "function",
    }
}
