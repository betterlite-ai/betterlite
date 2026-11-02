use crate::message_content::Message;

// Consecutive user-type messages become one user turn on the wire, so a block is the unit a system message can validly follow.
pub struct UserTypeBlock {
    pub is_user: bool,
    pub indices: Vec<usize>,
}

#[must_use]
pub fn user_type_blocks(messages: &[Message]) -> Vec<UserTypeBlock> {
    let indexed: Vec<(usize, &Message)> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role != "system")
        .collect();
    indexed
        .chunk_by(|(_, left), (_, right)| {
            matches!(left.role.as_str(), "user" | "tool" | "function")
                == matches!(right.role.as_str(), "user" | "tool" | "function")
        })
        .map(|chunk| UserTypeBlock {
            is_user: chunk.first().is_some_and(|(_, message)| {
                matches!(message.role.as_str(), "user" | "tool" | "function")
            }),
            indices: chunk.iter().map(|(index, _)| *index).collect(),
        })
        .collect()
}

#[must_use]
pub fn system_runs(messages: &[Message]) -> Vec<Vec<usize>> {
    let system_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == "system")
        .map(|(index, _)| index)
        .collect();
    system_indices
        .chunk_by(|&left, &right| right == left.saturating_add(1))
        .map(<[usize]>::to_vec)
        .collect()
}

mod tests;
