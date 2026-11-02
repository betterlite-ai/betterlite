use crate::message_content::Message;
use crate::system_placement::kinds::{MessageKind, kind};
use crate::system_placement::text_parts::converted_user_turns;

struct Run {
    kind: MessageKind,
    messages: Vec<Message>,
}

fn runs(messages: &[Message]) -> Vec<Run> {
    messages
        .chunk_by(|left, right| kind(left) == kind(right))
        .map(|chunk| Run {
            kind: chunk.first().map_or(MessageKind::Other, kind),
            messages: chunk.to_vec(),
        })
        .collect()
}

fn previous_run_is_system(runs: &[Run], index: usize) -> bool {
    index > 0
        && runs
            .get(index.saturating_sub(1))
            .is_some_and(|previous| previous.kind == MessageKind::System)
}

// Convert every system message to a user turn in place, except a run right before a tool result, which is emitted after that tool run.
#[must_use]
pub fn converted_for_unflagged_model(messages: &[Message]) -> Vec<Message> {
    let runs = runs(messages);
    let mut result = Vec::with_capacity(messages.len());
    for (index, run) in runs.iter().enumerate() {
        match run.kind {
            MessageKind::System => {
                let follower_is_tool = runs
                    .get(index.saturating_add(1))
                    .is_some_and(|follower| follower.kind == MessageKind::Tool);
                if !follower_is_tool {
                    result.extend(converted_user_turns(&run.messages));
                }
            }
            MessageKind::Tool if previous_run_is_system(&runs, index) => {
                result.extend_from_slice(&run.messages);
                if let Some(previous) = runs.get(index.saturating_sub(1)) {
                    result.extend(converted_user_turns(&previous.messages));
                }
            }
            MessageKind::Tool | MessageKind::User | MessageKind::Other => {
                result.extend_from_slice(&run.messages);
            }
        }
    }
    result
}

mod tests;
