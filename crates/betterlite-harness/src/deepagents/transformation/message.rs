use rust_decimal::Decimal;

use crate::events::Event;
use crate::types::JsonObject;

use super::tools::{BUILTIN_TOOLS, normalized_tool_name};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MessageKind {
    Ai,
    AiMessageChunk,
    Tool,
    Human,
    System,
    #[default]
    Other,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Content {
    #[default]
    Empty,
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentBlock {
    Text(String),
    Reasoning(String),
    Other(JsonObject),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MessageUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: JsonObject,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Message {
    pub kind: MessageKind,
    pub content: Content,
    // LangChain keeps reasoning deltas in additional_kwargs.reasoning_content
    pub reasoning_content: String,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: String,
    pub name: Option<String>,
    pub is_error: bool,
    pub usage: MessageUsage,
    // LangChain's response_metadata.response_cost when litellm already priced the call
    pub reported_cost: Option<Decimal>,
}

#[must_use]
pub fn content_text(content: &Content) -> String {
    match content {
        Content::Empty => String::new(),
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.as_str()),
                ContentBlock::Reasoning(_) | ContentBlock::Other(_) => None,
            })
            .collect(),
    }
}

#[must_use]
pub fn reasoning_text(message: &Message) -> String {
    if !message.reasoning_content.is_empty() {
        return message.reasoning_content.clone();
    }
    let Content::Blocks(blocks) = &message.content else {
        return String::new();
    };
    let mut reasoning = String::new();
    for block in blocks {
        if let ContentBlock::Reasoning(text) = block {
            reasoning.push_str(text);
        }
    }
    reasoning
}

#[must_use]
pub fn stream_events(message: &Message) -> Vec<Event> {
    if !matches!(message.kind, MessageKind::Ai | MessageKind::AiMessageChunk) {
        return Vec::new();
    }
    let mut events = Vec::new();
    let reasoning = reasoning_text(message);
    if !reasoning.is_empty() {
        events.push(Event::Reasoning { delta: reasoning });
    }
    let text = content_text(&message.content);
    if !text.is_empty() {
        events.push(Event::Text { delta: text });
    }
    events
}

#[must_use]
pub fn tool_call_event(call: &ToolCall) -> Event {
    Event::ToolCall {
        id: call.id.clone(),
        name: normalized_tool_name(&call.name).to_owned(),
        native_name: call.name.clone(),
        input: call.args.clone(),
        builtin: BUILTIN_TOOLS.contains(&call.name.as_str()),
    }
}

#[must_use]
pub fn final_ai_text(messages: &[Message]) -> String {
    for message in messages.iter().rev() {
        if message.kind != MessageKind::Ai {
            continue;
        }
        let text = content_text(&message.content);
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}
