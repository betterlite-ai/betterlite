use betterlite_prompts::message_content::Message;

use crate::cache_prefix::marking::is_cached_message;

const SYSTEM: &str = "system";

pub fn separate_cached_messages(
    messages: &[Message],
    disable_transform: bool,
) -> (Vec<Message>, Vec<Message>) {
    let marked: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| is_cached_message(message, disable_transform))
        .map(|(index, _)| index)
        .collect();
    let Some((&first, rest)) = marked.split_first() else {
        return (Vec::new(), messages.to_vec());
    };
    let last = first_run_end(rest, first);
    let cached = messages
        .get(first..=last)
        .map(<[Message]>::to_vec)
        .unwrap_or_default();
    let mut non_cached = messages
        .get(..first)
        .map(<[Message]>::to_vec)
        .unwrap_or_default();
    non_cached.extend_from_slice(messages.get(last.saturating_add(1)..).unwrap_or_default());
    (cached, non_cached)
}

fn first_run_end(rest: &[usize], first: usize) -> usize {
    let mut previous = first;
    for index in rest {
        if *index != previous.saturating_add(1) {
            return previous;
        }
        previous = *index;
    }
    previous
}

#[must_use]
pub fn ends_on_supported_turn(cached_messages: &[Message]) -> bool {
    let terminal = cached_messages
        .iter()
        .rev()
        .find(|message| message.role != SYSTEM);
    terminal.map_or(!cached_messages.is_empty(), |message| {
        !matches!(message.role.as_str(), "assistant" | "tool" | "function")
    })
}
