use super::config::normalize_tool_name;
use crate::events::Event;
use crate::payloads::python_json::python_json_text;
use crate::payloads::{last_json_object, stderr_tail_text};
use crate::types::JsonObject;
use std::sync::Arc;
use wire::{AssistantLine, Block, Line, MessageContent, ResultLine, SystemLine, UserLine};

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("invalid {kind} event shape")]
    Shape {
        kind: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Clone, Debug, Default)]
pub struct ClaudeCodeStreamState {
    pub session_id: Option<String>,
    pub text_parts: Vec<String>,
    pub result_seen: bool,
    pub result_text: Option<String>,
    pub is_error: bool,
    pub errors: Vec<String>,
    pub structured_output: Option<JsonObject>,
    pub parse_errors: Vec<Arc<StreamError>>,
}

impl ClaudeCodeStreamState {
    #[must_use]
    pub fn final_text(&self) -> String {
        if let Some(text) = &self.result_text {
            return text.clone();
        }
        self.text_parts.join("")
    }

    #[must_use]
    pub fn structured_output_json(&self) -> Option<String> {
        if let Some(object) = self.structured_output.as_ref()
            && let Ok(text) = python_json_text(object)
        {
            return Some(text);
        }
        last_json_object(&self.final_text())
    }
}

pub fn transform(line: &JsonObject, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    match wire::parse(line) {
        Ok(Line::Assistant(assistant)) => assistant_events(assistant, state),
        Ok(Line::User(user)) => user_events(user, state),
        Ok(Line::System(system)) => system_events(system, state),
        Ok(Line::Result(result)) => result_events(result, state),
        Ok(Line::Other) => Vec::new(),
        Err(error) => {
            state.parse_errors.push(Arc::new(error));
            Vec::new()
        }
    }
}

fn assistant_events(assistant: AssistantLine, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    if assistant
        .parent_tool_use_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
    {
        return Vec::new();
    }
    if assistant.message.model.as_deref() == Some(wire::SYNTHETIC_MODEL) {
        return Vec::new();
    }
    match assistant.message.content {
        MessageContent::Text(text) => text_block_event(text, state).into_iter().collect(),
        MessageContent::Blocks(blocks) => blocks
            .into_iter()
            .filter_map(|value| {
                let block = parse_block(&value, state)?;
                block_event(block, state)
            })
            .collect(),
        MessageContent::Other(_) => Vec::new(),
    }
}

fn text_block_event(text: String, state: &mut ClaudeCodeStreamState) -> Option<Event> {
    if text.is_empty() {
        return None;
    }
    state.text_parts.push(text.clone());
    Some(Event::Text { delta: text })
}

fn block_event(block: Block, state: &mut ClaudeCodeStreamState) -> Option<Event> {
    match block {
        Block::Text { text } => text_block_event(text, state),
        Block::Thinking { thinking } if !thinking.is_empty() => {
            Some(Event::Reasoning { delta: thinking })
        }
        Block::ToolUse { id, name, input } => Some(Event::ToolCall {
            id,
            name: normalize_tool_name(&name),
            builtin: !name.starts_with("mcp__"),
            native_name: name,
            input: input.as_ref().map(wire::object).unwrap_or_default(),
        }),
        Block::Thinking { .. } | Block::ToolResult { .. } | Block::Other => None,
    }
}

fn parse_block(value: &wire::WireValue, state: &mut ClaudeCodeStreamState) -> Option<Block> {
    match wire::block(value) {
        Ok(block) => Some(block),
        Err(error) => {
            state.parse_errors.push(Arc::new(error));
            None
        }
    }
}

fn user_events(user: UserLine, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    if user
        .parent_tool_use_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
    {
        return Vec::new();
    }
    let MessageContent::Blocks(blocks) = user.message.content else {
        return Vec::new();
    };
    blocks
        .into_iter()
        .filter_map(|value| parse_block(&value, state))
        .filter_map(|block| match block {
            Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some(Event::ToolResult {
                id: tool_use_id,
                output: wire::stringify_tool_output(&content),
                is_error,
            }),
            Block::Text { .. } | Block::Thinking { .. } | Block::ToolUse { .. } | Block::Other => {
                None
            }
        })
        .collect()
}

fn system_events(system: SystemLine, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    if system.subtype.as_deref() == Some("init") {
        if let Some(id) = system.session_id.filter(|id| !id.is_empty()) {
            state.session_id = Some(id);
        }
        return Vec::new();
    }
    if system.subtype.as_deref() == Some("compact_boundary") {
        return vec![Event::Compaction {
            tokens_before: system.compact_metadata.and_then(|meta| meta.pre_tokens),
            tokens_after: None,
        }];
    }
    Vec::new()
}

fn result_events(result: ResultLine, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    state.result_seen = true;
    state.is_error = result.is_error;
    state.result_text = result.result.and_then(|value| match value {
        wire::WireValue::Text(text) => Some(text),
        wire::WireValue::Null
        | wire::WireValue::Bool(_)
        | wire::WireValue::Number(_)
        | wire::WireValue::Array(_)
        | wire::WireValue::Object(_) => None,
    });
    state.errors = result.errors.unwrap_or_default();
    state.structured_output = result
        .structured_output
        .and_then(wire::StructuredOutputField::into_dict);
    if let Some(id) = result.session_id.filter(|id| !id.is_empty()) {
        state.session_id = Some(id);
    }
    Vec::new()
}

pub(super) fn turn_error(
    state: &ClaudeCodeStreamState,
    exit_code: i32,
    stderr_tail: &[String],
) -> Option<String> {
    if exit_code == 0_i32 && state.result_seen && !state.is_error && state.parse_errors.is_empty() {
        return None;
    }
    let reason = (!state.parse_errors.is_empty())
        .then(|| {
            state
                .parse_errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })
        .or_else(|| state.result_text.clone().filter(|text| !text.is_empty()))
        .or_else(|| (!state.errors.is_empty()).then(|| state.errors.join("; ")))
        .filter(|reason| !reason.is_empty())
        .unwrap_or_else(|| {
            if state.result_seen {
                "unknown error".to_owned()
            } else {
                "no result event".to_owned()
            }
        });
    let message = format!("claude exited with code {exit_code}: {reason}");
    let tail = stderr_tail_text(stderr_tail);
    Some(if tail.is_empty() {
        message
    } else {
        format!("{message}\nstderr:\n{tail}")
    })
}

#[path = "wire.rs"]
mod wire;
