use crate::message_content::{Content, Message};

fn carries_tool_reference(message: &Message) -> bool {
    let Some(Content::Parts(parts)) = &message.content else {
        return false;
    };
    message.role == "tool" && parts.iter().any(|part| part.kind == "tool_reference")
}

fn strip_tool_reference(message: &Message) -> Message {
    let Some(Content::Parts(parts)) = &message.content else {
        return message.clone();
    };
    if !carries_tool_reference(message) {
        return message.clone();
    }
    let remaining: Vec<_> = parts
        .iter()
        .filter(|&part| part.kind != "tool_reference")
        .cloned()
        .collect();
    let mut rewritten = message.clone();
    rewritten.content = Some(if remaining.is_empty() {
        Content::Text(String::new())
    } else {
        Content::Parts(remaining)
    });
    rewritten
}

// A reference-only tool message keeps empty text so the preceding tool_call stays answered.
#[must_use]
pub fn drop_tool_reference_parts(messages: &[Message]) -> Vec<Message> {
    if !messages.iter().any(carries_tool_reference) {
        return messages.to_vec();
    }
    messages.iter().map(strip_tool_reference).collect()
}

mod tests;
