use betterlite_models::decisions::Json;

use crate::stream_state::json_fragment_buffer::{AccumulatedJson as _, JsonFragmentBuffer};

pub(crate) fn could_close_json(buffer: &JsonFragmentBuffer) -> bool {
    buffer
        .snapshot()
        .chars()
        .rev()
        .find(|character| !character.is_whitespace())
        .is_some_and(|character| character == '}' || character == ']')
}

pub(crate) fn pop_next_value(buffer: &mut JsonFragmentBuffer) -> Option<Json> {
    let text = buffer.snapshot();
    let trimmed = text.trim_start();
    let mut values = serde_json::Deserializer::from_str(trimmed).into_iter::<Json>();
    let value = values.next()?.ok()?;
    let (_, tail) = trimmed.split_at_checked(values.byte_offset())?;
    buffer.set(tail.to_owned());
    Some(value)
}
