use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use super::error::QualityRouterError;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RequestText {
    pub user_message: Option<String>,
    pub system_prompt: Option<String>,
}

impl RequestText {
    // the hook reads the newest user message and the newest system prompt, whichever names its role first
    pub fn extract(messages: &[Message]) -> Result<Self, QualityRouterError> {
        let mut text = Self::default();
        for message in messages.iter().rev() {
            let content = message_text(message)?;
            if content.is_empty() {
                continue;
            }
            match message.role.as_str() {
                "user" if text.user_message.is_none() => text.user_message = Some(content),
                "system" if text.system_prompt.is_none() => text.system_prompt = Some(content),
                _ => {}
            }
        }
        Ok(text)
    }
}

fn message_text(message: &Message) -> Result<String, QualityRouterError> {
    match &message.content {
        None => Ok(String::new()),
        Some(Content::Text(text)) => Ok(text.clone()),
        Some(Content::Parts(parts)) => parts_text(&message.role, parts),
    }
}

// only text parts of a content list carry the prompt; their text is joined with one space and stripped
fn parts_text(role: &str, parts: &[ContentPart]) -> Result<String, QualityRouterError> {
    let mut pieces = Vec::new();
    for (position, part) in parts.iter().enumerate() {
        if part.kind != "text" {
            continue;
        }
        match part.fields.get("text") {
            None => pieces.push(String::new()),
            Some(Json::Text(text)) => pieces.push(text.clone()),
            Some(_) => {
                return Err(QualityRouterError::PartTextNotString {
                    role: role.to_owned(),
                    part: position,
                });
            }
        }
    }
    Ok(pieces.join(" ").trim().to_owned())
}
