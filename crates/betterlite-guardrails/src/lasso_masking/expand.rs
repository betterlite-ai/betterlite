use crate::bedrock_message_selection::wire::{Content, Part, Role};
use crate::lasso::{self, wire as lasso_wire};
use serde_json::value::RawValue;

use super::wire::{
    LassoContent, LassoMessage, Message, ToolArguments, ToolCall, ToolResultBlock, ToolUseBlock,
};

// A tool message becomes a developer/tool_result block, an assistant tool_calls entry becomes its own model/tool_use block, and plain text passes through as-is.
#[must_use]
pub fn expand_for_classification(messages: &[Message]) -> Vec<LassoMessage> {
    let mut expanded = Vec::new();
    for message in messages {
        expand_message(message, &mut expanded);
    }
    expanded
}

fn expand_message(message: &Message, expanded: &mut Vec<LassoMessage>) {
    if message.role == Role::Tool {
        expand_tool_message(message, expanded);
        return;
    }
    if let Some(text) = flatten_text(message.content.as_ref()) {
        expanded.push(LassoMessage {
            role: role_name(message.role),
            content: LassoContent::Text(text),
        });
    }
    if message.role == Role::Assistant {
        expand_tool_calls(message, expanded);
    }
}

fn expand_tool_message(message: &Message, expanded: &mut Vec<LassoMessage>) {
    let Some(tool_use_id) = message.tool_call_id.clone() else {
        return;
    };
    let block = ToolResultBlock {
        kind: "tool_result".to_owned(),
        tool_use_id: Some(tool_use_id),
        content: Some(flatten_tool_result_text(message.content.as_ref())),
    };
    expanded.push(LassoMessage {
        content: LassoContent::ToolResult(block),
        role: "developer".to_owned(),
    });
}

fn expand_tool_calls(message: &Message, expanded: &mut Vec<LassoMessage>) {
    for call in message.tool_calls.as_deref().unwrap_or_default() {
        push_tool_use(call, expanded);
    }
}

fn push_tool_use(call: &ToolCall, expanded: &mut Vec<LassoMessage>) {
    let fields = extract_tool_call_fields(call);
    let (Some(id), Some(name)) = (
        fields.id.filter(|id| !id.is_empty()),
        fields.name.filter(|name| !name.is_empty()),
    ) else {
        return;
    };
    expanded.push(LassoMessage {
        role: "model".to_owned(),
        content: LassoContent::ToolUse(ToolUseBlock {
            kind: "tool_use".to_owned(),
            id: Some(id),
            name: Some(name),
            input: fields.input.map(convert_tool_arguments),
        }),
    });
}

fn convert_tool_arguments(arguments: lasso_wire::ToolArguments) -> ToolArguments {
    match arguments {
        lasso_wire::ToolArguments::Parsed(value) => serde_json::to_string(&value)
            .ok()
            .and_then(|raw| RawValue::from_string(raw).ok())
            .map_or_else(
                || ToolArguments::Malformed {
                    arguments: String::new(),
                },
                ToolArguments::Parsed,
            ),
        lasso_wire::ToolArguments::Malformed {
            arguments: raw_arguments,
        } => ToolArguments::Malformed {
            arguments: raw_arguments,
        },
    }
}

fn extract_tool_call_fields(call: &ToolCall) -> lasso_wire::ToolCallFields {
    let converted = lasso_wire::ToolCall {
        id: call.id.clone(),
        function: call
            .function
            .as_ref()
            .map(|function| lasso_wire::ToolCallFunction {
                name: function.name.clone(),
                arguments: function.arguments.clone(),
            }),
    };
    lasso::extract_tool_call_fields(&converted)
}

// None for absent or empty content; otherwise the text, flattening a multimodal part list to its text-typed parts joined with newlines.
fn flatten_text(content: Option<&Content>) -> Option<String> {
    match content {
        None => None,
        Some(Content::Text(text)) => (!text.is_empty()).then(|| text.clone()),
        Some(Content::List(parts)) => {
            let joined = join_text_parts(parts);
            (!joined.is_empty()).then_some(joined)
        }
    }
}

// Same flattening, but a tool message always emits a (possibly empty) string: Lasso's tool_result.content field is required.
fn flatten_tool_result_text(content: Option<&Content>) -> String {
    match content {
        None => String::new(),
        Some(Content::Text(text)) => text.clone(),
        Some(Content::List(parts)) => join_text_parts(parts),
    }
}

fn join_text_parts(parts: &[Part]) -> String {
    let mut texts = Vec::new();
    for part in parts {
        if let Some(text) = part_text(part) {
            texts.push(text);
        }
    }
    texts.join("\n")
}

fn part_text(part: &Part) -> Option<String> {
    let Part::Block(block) = part else {
        return None;
    };
    if block.kind.as_deref() != Some("text") {
        return None;
    }
    block.text.clone().filter(|text| !text.is_empty())
}

fn role_name(role: Role) -> String {
    match role {
        Role::System => "system".to_owned(),
        Role::Developer => "developer".to_owned(),
        Role::User => "user".to_owned(),
        Role::Assistant => "assistant".to_owned(),
        Role::Tool => "tool".to_owned(),
        Role::Function => "function".to_owned(),
    }
}
