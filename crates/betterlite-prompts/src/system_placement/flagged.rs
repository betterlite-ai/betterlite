use crate::message_content::Message;
use crate::system_placement::anchor::anchor_block;
use crate::system_placement::blocks::{system_runs, user_type_blocks};
use crate::system_placement::text_parts::{converted_user_turns, merged_system_message};

struct Anchor {
    run: Vec<usize>,
    block_index: Option<usize>,
}

fn messages_of(messages: &[Message], run: &[usize]) -> Vec<Message> {
    run.iter()
        .filter_map(|&index| messages.get(index).cloned())
        .collect()
}

fn anchored_messages(anchors: &[Anchor], messages: &[Message], block_index: usize) -> Vec<Message> {
    anchors
        .iter()
        .filter(|anchor| anchor.block_index == Some(block_index))
        .flat_map(|anchor| messages_of(messages, &anchor.run))
        .collect()
}

// A run whose anchor search failed converts in place right after the message that follows it.
fn converted_after(anchors: &[Anchor], messages: &[Message], message_index: usize) -> Vec<Message> {
    let following_index = message_index.saturating_add(1);
    anchors
        .iter()
        .filter(|anchor| {
            anchor.block_index.is_none() && anchor.run.first() == Some(&following_index)
        })
        .flat_map(|anchor| converted_user_turns(&messages_of(messages, &anchor.run)))
        .collect()
}

// Keep system messages as role: "system" at a placement Anthropic accepts, sliding a run that follows an assistant turn behind the next user turn and merging runs that land on the same turn.
#[must_use]
pub fn placed_for_flagged_model(messages: &[Message]) -> Vec<Message> {
    let blocks = user_type_blocks(messages);
    let anchors: Vec<Anchor> = system_runs(messages)
        .into_iter()
        .map(|run| {
            let block_index = anchor_block(&run, messages, &blocks);
            Anchor { run, block_index }
        })
        .collect();

    let mut result = Vec::with_capacity(messages.len());
    for (block_index, block) in blocks.iter().enumerate() {
        for &index in &block.indices {
            if let Some(message) = messages.get(index) {
                result.push(message.clone());
            }
            result.extend(converted_after(&anchors, messages, index));
        }
        if block.is_user {
            let anchored = anchored_messages(&anchors, messages, block_index);
            result.extend(merged_system_message(&anchored));
        }
    }
    result
}

mod tests;
