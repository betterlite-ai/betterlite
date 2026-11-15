use crate::stream_base_assembly::response::BaseResponse;

use super::records::{Choice, Chunk, Delta};

// The fast path: every choice delta is text-only, so the joined nonempty parts are the message text.
pub(crate) fn apply(base: &mut BaseResponse, chunks: &[Chunk]) -> bool {
    let mut parts: Vec<&str> = Vec::new();
    for chunk in chunks {
        if chunk.choices.is_empty() {
            continue;
        }
        let Some(text) = simple_text_part(&chunk.choices) else {
            return false;
        };
        if !text.is_empty() {
            parts.push(text);
        }
    }
    if !parts.is_empty()
        && let Some(choice) = base.choices.first_mut()
    {
        choice.message.content = Some(parts.concat());
    }
    true
}

// A choice without a delta reads as an empty mapping, and a text choice never carries one.
fn simple_text_part(choices: &[Choice]) -> Option<&str> {
    let mut first_content = None;
    for (index, choice) in choices.iter().enumerate() {
        let delta = choice.delta.as_ref();
        if delta.is_some_and(Delta::carries_more_than_text) {
            return None;
        }
        if index == 0 {
            first_content = delta.and_then(|choice_delta| choice_delta.content.as_deref());
        }
    }
    Some(first_content.unwrap_or_default())
}
