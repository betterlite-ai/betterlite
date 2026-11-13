use crate::arguments::json::{Json, Object};
use crate::cache_accounting::constants::CACHE_BREAKPOINT_KEYS;
use crate::cache_placement::system::SystemPrompt;
use crate::message_content::{Content, Message};

// The key has to hold a value: an explicit null is no breakpoint, as an explicit null is no cache_control.
#[must_use]
pub fn fields_carry_mark(fields: &Object) -> bool {
    CACHE_BREAKPOINT_KEYS.iter().any(|key| {
        fields
            .get(*key)
            .is_some_and(|value| !matches!(value, Json::Null))
    })
}

#[must_use]
pub fn count_message_marks(message: &Message) -> usize {
    let parts = match message.content.as_ref() {
        Some(Content::Parts(parts)) => parts.as_slice(),
        Some(Content::Text(_)) | None => &[],
    };
    let marked = parts
        .iter()
        .filter(|part| fields_carry_mark(&part.fields))
        .count();
    usize::from(fields_carry_mark(&message.provider_specific_fields)).saturating_add(marked)
}

#[must_use]
pub fn system_has_mark(system: &SystemPrompt) -> bool {
    match system {
        SystemPrompt::Blocks(blocks) => blocks.iter().any(fields_carry_mark),
        SystemPrompt::Text(_) => false,
    }
}

#[must_use]
pub fn count_system_marks(system: Option<&SystemPrompt>) -> usize {
    let Some(SystemPrompt::Blocks(blocks)) = system else {
        return 0;
    };
    blocks
        .iter()
        .filter(|block| fields_carry_mark(block))
        .count()
}

#[must_use]
pub fn count_request_marks(messages: &[Message], system: Option<&SystemPrompt>) -> usize {
    messages
        .iter()
        .map(count_message_marks)
        .sum::<usize>()
        .saturating_add(count_system_marks(system))
}
