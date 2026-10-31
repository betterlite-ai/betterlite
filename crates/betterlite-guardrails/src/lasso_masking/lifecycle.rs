use crate::bedrock_message_selection::wire::{Content, Role};
use crate::lasso::wire::MessageType;

use super::map_back::map_masked_messages_back;
use super::wire::{LassoMessage, Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Classify,
    Classifix,
}

#[derive(Debug, Clone)]
pub struct LifecycleResult {
    pub endpoint: Endpoint,
    pub message_type: MessageType,
    pub inspected: Vec<LassoMessage>,
    pub messages: Vec<Message>,
    pub input: Option<String>,
}

// This is the pure lifecycle seam: the transport owner supplies the classifix result, while this function owns endpoint selection and write-back.
#[must_use]
pub fn run(
    messages: &[Message],
    input: Option<&str>,
    mask: bool,
    message_type: MessageType,
    masked: Option<&[LassoMessage]>,
) -> LifecycleResult {
    let mut inspected = super::expand::expand_for_classification(messages);
    let message_count = inspected.len();
    append_input(&mut inspected, input);
    let endpoint = if mask && !has_non_string_content(messages) {
        Endpoint::Classifix
    } else {
        Endpoint::Classify
    };
    let Some(masked_messages) = masked.filter(|_| endpoint == Endpoint::Classifix) else {
        return LifecycleResult {
            endpoint,
            message_type,
            inspected,
            messages: messages.to_vec(),
            input: input.map(str::to_owned),
        };
    };
    let (message_mask, input_mask) =
        masked_messages.split_at(masked_messages.len().min(message_count));
    let updated_input = masked_input(input_mask);
    LifecycleResult {
        endpoint,
        message_type,
        inspected,
        messages: map_masked_messages_back(messages, message_mask),
        input: input.map(|original| {
            if updated_input.is_empty() {
                original.to_owned()
            } else {
                updated_input.join("\n")
            }
        }),
    }
}

fn append_input(inspected: &mut Vec<LassoMessage>, input: Option<&str>) {
    if let Some(input_text) = input {
        inspected.push(LassoMessage {
            role: "user".to_owned(),
            content: super::wire::LassoContent::Text(input_text.to_owned()),
        });
    }
}

fn masked_input(messages: &[LassoMessage]) -> Vec<&str> {
    if messages.is_empty() {
        return Vec::new();
    }
    messages
        .iter()
        .filter_map(|message| match &message.content {
            super::wire::LassoContent::Text(text) => Some(text.as_str()),
            super::wire::LassoContent::ToolUse(_) | super::wire::LassoContent::ToolResult(_) => {
                None
            }
        })
        .collect()
}

fn has_non_string_content(messages: &[Message]) -> bool {
    if messages.is_empty() {
        return false;
    }
    messages.iter().any(|message| {
        message.role != Role::Tool && matches!(message.content, Some(Content::List(_)))
    })
}
