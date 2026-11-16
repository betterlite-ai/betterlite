use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};

use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::MarkerPair;
use crate::classifier_message_text::text::{message_text, strip_reminder_blocks};

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

// one message's role and human text, so a typed history and a raw body's messages share the scans
pub trait ClassifierMessage {
    fn classifier_role(&self) -> Option<&str>;
    fn human_text(&self, markers: &[MarkerPair<'_>]) -> Result<String, ClassifierTextError>;
}

impl ClassifierMessage for ChatMessage {
    fn classifier_role(&self) -> Option<&str> {
        Some(role_name(self.role))
    }

    fn human_text(&self, markers: &[MarkerPair<'_>]) -> Result<String, ClassifierTextError> {
        Ok(strip_reminder_blocks(
            &message_text(self.content.as_ref())?,
            markers,
        ))
    }
}

impl ClassifierMessage for Json {
    fn classifier_role(&self) -> Option<&str> {
        json_role(self)
    }

    fn human_text(&self, markers: &[MarkerPair<'_>]) -> Result<String, ClassifierTextError> {
        Ok(strip_reminder_blocks(
            &message_text(json_content(self))?,
            markers,
        ))
    }
}

pub fn human_asks_newest_first<T>(
    messages: &[T],
    markers: &[MarkerPair<'_>],
) -> Result<Vec<ExtractedAsk>, ClassifierTextError>
where
    T: ClassifierMessage,
{
    let mut asks: Vec<ExtractedAsk> = Vec::new();
    for (index, message) in messages.iter().enumerate().rev() {
        if message.classifier_role() != Some("user") {
            continue;
        }
        let text = message.human_text(markers)?;
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
    messages.is_none_or(|history| {
        history.is_empty()
            || history
                .iter()
                .any(|message| message.role == Role::Assistant)
    })
}

pub fn newest_turn_ask<T>(
    messages: &[T],
    markers: &[MarkerPair<'_>],
) -> Result<Option<ExtractedAsk>, ClassifierTextError>
where
    T: ClassifierMessage,
{
    let Some((index, message)) = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.classifier_role() == Some("user"))
    else {
        return Ok(None);
    };
    let text = message.human_text(markers)?;
    Ok((!text.is_empty()).then_some(ExtractedAsk {
        text,
        message_index: index,
    }))
}

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

pub fn newest_turn_is_human_ask(
    messages: Option<&[ChatMessage]>,
    markers: &[MarkerPair<'_>],
) -> Result<bool, ClassifierTextError> {
    let Some(history) = messages.filter(|candidate| !candidate.is_empty()) else {
        return Ok(false);
    };
    let mut newest_activity = None;
    for (index, message) in history.iter().enumerate().rev() {
        if message.role != Role::System && !is_reminder_only_turn(message, markers)? {
            newest_activity = Some(index);
            break;
        }
    }
    let Some(activity) = newest_activity else {
        return Ok(false);
    };
    let newest_ask = human_asks_newest_first(history, markers)?
        .first()
        .map(|ask| ask.message_index);
    Ok(newest_ask == Some(activity))
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
