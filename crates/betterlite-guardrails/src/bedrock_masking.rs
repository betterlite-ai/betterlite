use crate::bedrock_message_selection::wire::{Content, Message, ModelResponse, Part};
use crate::bedrock_records::Response;
use std::slice::Iter;

mod tests;

#[must_use]
pub fn extract_masked_texts(response: Option<&Response>) -> Vec<String> {
    let outputs = response.and_then(|item| item.outputs.as_ref());
    outputs
        .into_iter()
        .flatten()
        .filter_map(|output| output.text.clone())
        .collect()
}

#[must_use]
pub fn apply_to_messages(messages: &[Message], masked_texts: &[String]) -> Vec<Message> {
    let mut remaining = masked_texts.iter();
    let mut updated = messages.to_vec();
    for message in &mut updated {
        match &mut message.content {
            Some(Content::Text(text)) => {
                if let Some(masked) = remaining.next() {
                    text.clone_from(masked);
                }
            }
            Some(Content::List(parts)) => parts.retain_mut(|part| mask_part(part, &mut remaining)),
            None => {}
        }
    }
    updated
}

fn mask_part(part: &mut Part, remaining: &mut Iter<'_, String>) -> bool {
    match part {
        Part::Block(block) if block.text.is_some() || block.extra.contains_key("text") => {
            if let Some(masked) = remaining.next() {
                block.text = Some(masked.clone());
                block.extra.remove("text");
            }
            true
        }
        Part::Text(text) => {
            if let Some(masked) = remaining.next() {
                text.clone_from(masked);
            }
            true
        }
        Part::Block(_) | Part::Other(_) => false,
    }
}

pub fn apply_to_response(response: &mut ModelResponse, masked_texts: &[String]) {
    let mut remaining = masked_texts.iter();
    for choice in response.choices.iter_mut().flatten() {
        let content = if let Some(message) = &mut choice.message {
            match &mut message.content {
                Some(Content::Text(text)) => Some(text),
                Some(Content::List(_)) | None => None,
            }
        } else if let Some(delta) = &mut choice.delta {
            match &mut delta.content {
                Some(Content::Text(text)) => Some(text),
                Some(Content::List(_)) | None => None,
            }
        } else {
            choice.text.as_mut()
        };
        if let Some(text) = content.filter(|text| !text.is_empty())
            && let Some(masked) = remaining.next()
        {
            text.clone_from(masked);
        }
    }
}

#[derive(Debug, Default)]
pub struct Masking {
    pub request: bool,
    pub response: bool,
}

#[must_use]
pub fn update_messages(messages: &[Message], response: &Response, flags: &Masking) -> Vec<Message> {
    let texts = extract_masked_texts(Some(response));
    if !texts.is_empty() {
        return apply_to_messages(messages, &texts);
    }
    if flags.request || flags.response {
        tracing::debug!("masking enabled but no masked output, returning original messages");
    }
    messages.to_vec()
}
