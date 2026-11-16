use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Running,
    Complete,
    Error,
}

impl ToolCallStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Status {
        message: String,
    },
    ToolCall {
        tool_name: String,
        tool_label: String,
        arguments: IndexMap<String, String>,
        status: ToolCallStatus,
    },
    Chunk {
        content: String,
    },
    Done,
    Error {
        message: String,
    },
}

#[must_use]
pub fn sse_frame(event: &SseEvent) -> String {
    let mut frame = String::from("data: ");
    push_event_json(&mut frame, event);
    frame.push_str("\n\n");
    frame
}

fn push_event_json(out: &mut String, event: &SseEvent) {
    match event {
        SseEvent::Status { message } => {
            out.push_str("{\"type\": \"status\", \"message\": ");
            push_json_string(out, message);
            out.push('}');
        }
        SseEvent::Chunk { content } => {
            out.push_str("{\"type\": \"chunk\", \"content\": ");
            push_json_string(out, content);
            out.push('}');
        }
        SseEvent::Done => out.push_str("{\"type\": \"done\"}"),
        SseEvent::Error { message } => {
            out.push_str("{\"type\": \"error\", \"message\": ");
            push_json_string(out, message);
            out.push('}');
        }
        SseEvent::ToolCall {
            tool_name,
            tool_label,
            arguments,
            status,
        } => {
            out.push_str("{\"type\": \"tool_call\", \"tool_name\": ");
            push_json_string(out, tool_name);
            out.push_str(", \"tool_label\": ");
            push_json_string(out, tool_label);
            out.push_str(", \"arguments\": {");
            for (index, (key, value)) in arguments.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                push_json_string(out, key);
                out.push_str(": ");
                push_json_string(out, value);
            }
            out.push_str("}, \"status\": ");
            push_json_string(out, status.as_str());
            out.push('}');
        }
    }
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        push_json_character(out, character);
    }
    out.push('"');
}

fn push_json_character(out: &mut String, character: char) {
    match character {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\u{08}' => out.push_str("\\b"),
        '\u{0c}' => out.push_str("\\f"),
        ' '..='~' => out.push(character),
        '\u{00}'..='\u{1f}' | '\u{7f}'..='\u{ffff}' => push_hex4(out, u32::from(character)),
        _ => push_surrogate_pair(out, character),
    }
}

fn push_hex4(out: &mut String, code: u32) {
    let _written = write!(out, "\\u{code:04x}");
}

fn push_surrogate_pair(out: &mut String, character: char) {
    let adjusted = u32::from(character).saturating_sub(0x1_0000);
    push_hex4(out, 0xd800 | (adjusted >> 10));
    push_hex4(out, 0xdc00 | (adjusted & 0x3ff));
}
