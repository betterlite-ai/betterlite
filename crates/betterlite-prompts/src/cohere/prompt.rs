use crate::cohere::error::CohereError;
use crate::cohere::limits::ArgumentBudget;
use crate::cohere::result::tool_result;
use crate::cohere::tool_calls::{Recovery, collect_tool_calls};
use crate::cohere::types::CoherePrompt;
use crate::media::limits::MAX_MESSAGES;
use crate::message_content::{Content, Message};

const TOOL: &str = "tool";
const SEPARATOR: &str = "\n\n";

pub fn cohere_message_pt(messages: &[Message]) -> Result<CoherePrompt, CohereError> {
    if messages.len() > MAX_MESSAGES {
        return Err(CohereError::TooManyMessages {
            limit: MAX_MESSAGES,
        });
    }
    let tool_calls = collect_tool_calls(messages)?;
    let mut recovery = Recovery {
        tool_calls: &tool_calls,
        budget: ArgumentBudget { spent: 0 },
    };
    let mut prompt = String::new();
    let mut tool_results = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role == TOOL {
            tool_results.push(tool_result(message, &mut recovery, index)?);
            continue;
        }
        match &message.content {
            Some(Content::Text(text)) if !text.is_empty() => {
                prompt.push_str(text);
                prompt.push_str(SEPARATOR);
            }
            Some(Content::Parts(parts)) if !parts.is_empty() => {
                return Err(CohereError::ContentNotText { index });
            }
            Some(Content::Text(_) | Content::Parts(_)) | None => {}
        }
    }
    Ok(CoherePrompt {
        prompt: prompt.trim_end().to_owned(),
        tool_results,
    })
}
