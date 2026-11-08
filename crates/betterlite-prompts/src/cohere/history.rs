use crate::cohere::content::{content_text, optional_content_text};
use crate::cohere::error::CohereError;
use crate::cohere::result::tool_result;
use crate::cohere::tool_calls::{
    collect_tool_calls, convert_to_cohere_tool_invoke, message_tool_calls,
};
use crate::cohere::types::{ChatHistoryEntry, CohereMessage, CohereTool, CohereV2Messages};
use crate::media::limits::MAX_MESSAGES;
use crate::message_content::Message;

const USER: &str = "user";
const SYSTEM: &str = "system";
const ASSISTANT: &str = "assistant";
const TOOL: &str = "tool";
const FUNCTION: &str = "function";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    User,
    System,
    Assistant,
    Tools,
}

impl Group {
    fn of(role: &str) -> Option<Self> {
        match role {
            USER => Some(Self::User),
            SYSTEM => Some(Self::System),
            ASSISTANT => Some(Self::Assistant),
            TOOL | FUNCTION => Some(Self::Tools),
            _ => None,
        }
    }
}

pub fn cohere_messages_pt_v2(messages: &[Message]) -> Result<CohereV2Messages, CohereError> {
    if messages.len() > MAX_MESSAGES {
        return Err(CohereError::TooManyMessages {
            limit: MAX_MESSAGES,
        });
    }
    let tool_calls = collect_tool_calls(messages)?;
    let (last, history) = messages.split_last().ok_or(CohereError::EmptyMessages)?;
    let index = messages.len().saturating_sub(1);
    let message = returned_message(last, &tool_calls, index)?;
    Ok(CohereV2Messages {
        message,
        chat_history: build_history(history, &tool_calls)?,
    })
}

fn returned_message(
    message: &Message,
    tool_calls: &[CohereTool],
    index: usize,
) -> Result<CohereMessage, CohereError> {
    if message.role == TOOL {
        return tool_result(message, tool_calls, index).map(CohereMessage::ToolResult);
    }
    content_text(message.content.as_ref(), index).map(CohereMessage::Text)
}

fn build_history(
    messages: &[Message],
    tool_calls: &[CohereTool],
) -> Result<Vec<ChatHistoryEntry>, CohereError> {
    let mut history = Vec::new();
    let mut position = 0_usize;
    while let Some(message) = messages.get(position) {
        let Some(group) = Group::of(&message.role) else {
            return Err(CohereError::UnsupportedRole {
                index: position,
                role: message.role.clone(),
            });
        };
        let end = run_end(messages, position, group);
        let run = messages.get(position..end).unwrap_or_default();
        push_entry(&mut history, group, run, position, tool_calls)?;
        position = end;
    }
    Ok(history)
}

fn run_end(messages: &[Message], start: usize, group: Group) -> usize {
    let mut end = start;
    while let Some(message) = messages.get(end) {
        if Group::of(&message.role) != Some(group) {
            break;
        }
        end = end.saturating_add(1);
    }
    end
}

fn push_entry(
    history: &mut Vec<ChatHistoryEntry>,
    group: Group,
    run: &[Message],
    offset: usize,
    tool_calls: &[CohereTool],
) -> Result<(), CohereError> {
    match group {
        Group::User => {
            let message = run_text(run, offset)?;
            if !message.is_empty() {
                history.push(ChatHistoryEntry::User { message });
            }
        }
        Group::System => {
            let message = run_text(run, offset)?;
            if !message.is_empty() {
                history.push(ChatHistoryEntry::System { message });
            }
        }
        Group::Assistant => {
            if let Some(entry) = assistant_entry(run, offset)? {
                history.push(entry);
            }
        }
        Group::Tools => history.push(tool_entry(run, offset, tool_calls)?),
    }
    Ok(())
}

fn run_text(run: &[Message], offset: usize) -> Result<String, CohereError> {
    let mut text = String::new();
    for (position, message) in run.iter().enumerate() {
        text.push_str(&content_text(
            message.content.as_ref(),
            offset.saturating_add(position),
        )?);
    }
    Ok(text)
}

fn assistant_entry(
    run: &[Message],
    offset: usize,
) -> Result<Option<ChatHistoryEntry>, CohereError> {
    let mut message = String::new();
    for (position, item) in run.iter().enumerate() {
        message.push_str(&optional_content_text(
            item.content.as_ref(),
            offset.saturating_add(position),
        )?);
    }
    let mut tool_calls = Vec::new();
    for (position, item) in run.iter().enumerate() {
        let tools = message_tool_calls(item, offset.saturating_add(position))?;
        tool_calls.extend(convert_to_cohere_tool_invoke(&tools)?);
    }
    // LiteLLM drops a run with no text, its tool calls included, and never turns function_call into one
    if message.is_empty() {
        return Ok(None);
    }
    Ok(Some(ChatHistoryEntry::Chatbot {
        message,
        tool_calls,
    }))
}

fn tool_entry(
    run: &[Message],
    offset: usize,
    tool_calls: &[CohereTool],
) -> Result<ChatHistoryEntry, CohereError> {
    let mut tool_results = Vec::with_capacity(run.len());
    for (position, message) in run.iter().enumerate() {
        tool_results.push(tool_result(
            message,
            tool_calls,
            offset.saturating_add(position),
        )?);
    }
    Ok(ChatHistoryEntry::Tool { tool_results })
}
