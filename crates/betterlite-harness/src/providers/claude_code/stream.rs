use super::config::normalize_tool_name;
use crate::events::Event;
use crate::payloads::python_json::python_json_text;
use crate::payloads::{last_json_object, stderr_tail_text};
use crate::types::JsonObject;
use wire::{AssistantLine, Block, Line, MessageContent, ResultLine, SystemLine, UserLine};

#[derive(Clone, Debug, Default)]
pub struct ClaudeCodeStreamState {
    pub session_id: Option<String>,
    pub text_parts: Vec<String>,
    pub result_seen: bool,
    pub result_text: Option<String>,
    pub is_error: bool,
    pub errors: Vec<String>,
    pub structured_output: Option<JsonObject>,
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
        Some(Line::Assistant(assistant)) => assistant_events(assistant, state),
        Some(Line::User(user)) => user_events(user),
        Some(Line::System(system)) => system_events(system, state),
        Some(Line::Result(result)) => result_events(result, state),
        Some(Line::Other) | None => Vec::new(),
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
            .filter_map(|value| wire::block(&value))
            .filter_map(|block| block_event(block, state))
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

fn user_events(user: UserLine) -> Vec<Event> {
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
        .filter_map(|value| wire::block(&value))
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
    if exit_code == 0_i32 && state.result_seen && !state.is_error {
        return None;
    }
    let reason = state
        .result_text
        .clone()
        .filter(|text| !text.is_empty())
        .or_else(|| (!state.errors.is_empty()).then(|| state.errors.join("; ")))
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
