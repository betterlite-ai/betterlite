use std::iter::once;

use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, ContentPart, Message};

#[must_use]
pub fn system_messages_first(messages: &[Message]) -> Vec<Message> {
    let is_instruction =
        |message: &Message| matches!(message.role.as_str(), "system" | "developer");
    let mut result = Vec::with_capacity(messages.len());
    result.extend(
        messages
            .iter()
            .filter(|message| is_instruction(message))
            .cloned(),
    );
    result.extend(
        messages
            .iter()
            .filter(|message| !is_instruction(message))
            .cloned(),
    );
    result
}

fn system_content_as_text_parts(content: &Content) -> Vec<ContentPart> {
    match content {
        Content::Text(text) => vec![ContentPart {
            kind: "text".to_owned(),
            fields: Object::from_iter([("text".to_owned(), Json::Text(text.clone()))]),
        }],
        Content::Parts(parts) => parts.clone(),
    }
}

const fn text_from_content(content: &Content) -> Option<&str> {
    match content {
        Content::Text(text) => Some(text.as_str()),
        Content::Parts(_) => None,
    }
}

fn merge_system_message_run(first: &Message, rest: &[Message]) -> Message {
    if rest.is_empty() {
        return first.clone();
    }
    let contents: Vec<&Content> = once(first)
        .chain(rest.iter())
        .filter_map(|message| message.content.as_ref())
        .collect();
    if contents.is_empty() {
        return first.clone();
    }
    let mut merged = first.clone();
    merged.content = Some(
        if contents
            .iter()
            .all(|content| matches!(content, Content::Text(_)))
        {
            let texts: Vec<&str> = contents
                .iter()
                .filter_map(|content| text_from_content(content))
                .collect();
            Content::Text(texts.join("\n\n"))
        } else {
            Content::Parts(
                contents
                    .iter()
                    .flat_map(|content| system_content_as_text_parts(content))
                    .collect(),
            )
        },
    );
    merged
}

#[must_use]
pub fn merge_adjacent_system_messages(messages: &[Message]) -> Vec<Message> {
    let mut result = Vec::with_capacity(messages.len());
    for run in messages.chunk_by(|left, right| (left.role == "system") == (right.role == "system"))
    {
        if let Some((first, rest)) = run.split_first() {
            if first.role == "system" {
                result.push(merge_system_message_run(first, rest));
            } else {
                result.extend_from_slice(run);
            }
        }
    }
    result
}

mod tests;
