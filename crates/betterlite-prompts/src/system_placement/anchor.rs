use crate::message_content::Message;
use crate::system_placement::blocks::UserTypeBlock;
use crate::system_placement::rendering::renders;

fn rendered_block(
    message_index: usize,
    messages: &[Message],
    blocks: &[UserTypeBlock],
) -> Option<usize> {
    let (block_index, block) = blocks
        .iter()
        .enumerate()
        .find(|(_, block)| block.indices.contains(&message_index))?;
    block
        .indices
        .iter()
        .any(|&index| messages.get(index).is_some_and(renders))
        .then_some(block_index)
}

// Whether a system message may follow this block is whether the first later block that renders anything is an assistant block.
fn system_may_follow(block_index: usize, messages: &[Message], blocks: &[UserTypeBlock]) -> bool {
    blocks
        .get(block_index.saturating_add(1)..)
        .unwrap_or(&[])
        .iter()
        .find(|block| {
            block
                .indices
                .iter()
                .any(|&index| messages.get(index).is_some_and(renders))
        })
        .is_none_or(|block| !block.is_user)
}

// The user-type block a system run must follow, or None when it converts in place; the run never starts at 0, since the caller splits off the leading system run first.
#[must_use]
pub fn anchor_block(
    run: &[usize],
    messages: &[Message],
    blocks: &[UserTypeBlock],
) -> Option<usize> {
    let first = *run.first()?;
    let previous = first.checked_sub(1)?;
    let previous_message = messages.get(previous)?;
    let neighbour = if matches!(previous_message.role.as_str(), "user" | "tool" | "function") {
        previous
    } else {
        run.last()?.checked_add(1)?
    };
    let neighbour_message = messages.get(neighbour)?;
    if !matches!(
        neighbour_message.role.as_str(),
        "user" | "tool" | "function"
    ) {
        return None;
    }
    let block_index = rendered_block(neighbour, messages, blocks)?;
    system_may_follow(block_index, messages, blocks).then_some(block_index)
}

mod tests;
