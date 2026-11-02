use super::encrypted_signature::is_encrypted_reasoning_block;
use crate::message_content::{Content, ContentPart, Message};

// A provider that did not mint an encrypted block rejects it signed or unsigned alike, so the whole block goes rather than falling back to unsigned text.
#[must_use]
pub fn strip_encrypted_reasoning(
    messages: &[Message],
    should_strip: Option<&dyn Fn(&ContentPart) -> bool>,
) -> Vec<Message> {
    messages
        .iter()
        .map(|message| strip_message(message, should_strip))
        .collect()
}

fn strip_message(
    message: &Message,
    should_strip: Option<&dyn Fn(&ContentPart) -> bool>,
) -> Message {
    let Some(Content::Parts(blocks)) = &message.content else {
        return message.clone();
    };
    let kept = blocks
        .iter()
        .filter(|block| {
            !is_encrypted_reasoning_block(block)
                || should_strip.is_some_and(|predicate| !predicate(block))
        })
        .cloned()
        .collect();
    let mut stripped = message.clone();
    stripped.content = Some(Content::Parts(kept));
    stripped
}

mod tests;
