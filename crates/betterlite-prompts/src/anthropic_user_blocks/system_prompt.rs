use thiserror::Error;

use crate::anthropic_history::tools::MAX_HISTORY_MESSAGES;
use crate::arguments::json::Json;

const CONTENT: &str = "content";
const ROLE: &str = "role";
const SYSTEM: &str = "system";

#[derive(Debug, Error)]
pub enum SystemPromptError {
    #[error("messages exceeds the maximum of {limit}")]
    TooManyMessages { limit: usize },
    #[error("message at index {index} is not an object")]
    MessageNotObject { index: usize },
    #[error("the content of system message at index {index} must be a string")]
    ContentNotText { index: usize },
}

pub fn get_system_prompt(messages: &[Json]) -> Result<(String, Vec<Json>), SystemPromptError> {
    if messages.len() > MAX_HISTORY_MESSAGES {
        return Err(SystemPromptError::TooManyMessages {
            limit: MAX_HISTORY_MESSAGES,
        });
    }
    let mut system_prompt = String::new();
    let mut remaining = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let Some(fields) = message.as_object() else {
            return Err(SystemPromptError::MessageNotObject { index });
        };
        if fields.get(ROLE).and_then(Json::as_str) != Some(SYSTEM) {
            remaining.push(message.clone());
            continue;
        }
        let Some(Json::Text(content)) = fields.get(CONTENT) else {
            return Err(SystemPromptError::ContentNotText { index });
        };
        system_prompt.push_str(content);
    }
    Ok((system_prompt, remaining))
}
