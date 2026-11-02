pub mod anchor;
pub mod anthropic_system;
pub mod blocks;
pub mod cache_control;
pub mod flagged;
pub mod kinds;
pub mod rendering;
pub mod text_parts;
pub mod unflagged;

use crate::message_content::Message;

// Only the leading run of system messages belongs in the top-level system parameter; a caller splits it off before running the rest of this policy on what remains.
#[must_use]
pub fn split_leading_system_run(messages: &[Message]) -> (Vec<Message>, Vec<Message>) {
    let leading_count = messages
        .iter()
        .position(|message| message.role != "system")
        .unwrap_or(messages.len());
    let (leading, rest) = messages.split_at(leading_count);
    (leading.to_vec(), rest.to_vec())
}

// Apply the placement policy to the messages after the leading system run: a flagged model keeps the role under Anthropic's placement rules, others get it converted to user turns.
#[must_use]
pub fn place_mid_conversation_system(
    messages: &[Message],
    supports_mid_convo_system: bool,
) -> Vec<Message> {
    if !messages.iter().any(|message| message.role == "system") {
        return messages.to_vec();
    }
    if supports_mid_convo_system {
        flagged::placed_for_flagged_model(messages)
    } else {
        unflagged::converted_for_unflagged_model(messages)
    }
}

mod tests;
