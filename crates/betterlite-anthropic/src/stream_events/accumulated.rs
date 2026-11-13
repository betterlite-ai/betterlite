use betterlite_models::decisions::Json;

use crate::stream_state::json_fragment_buffer::{AccumulatedJson as _, JsonFragmentBuffer};

// LiteLLM sets the flag from the fragment it appends, so a keepalive of blank text keeps the last byte seen.
pub(crate) fn updated_could_close(previous: bool, fragment: &str) -> bool {
    match fragment.trim_end().chars().next_back() {
        Some('}' | ']') => true,
        Some(_) => false,
        None => previous,
    }
}

// Values are peeled by advancing a cursor over one snapshot, and the unconsumed tail is written back once.
pub(crate) fn pop_next_value(
    buffer: &mut JsonFragmentBuffer,
    could_close: &mut bool,
) -> Option<Json> {
    let text = buffer.snapshot();
    let mut cursor = 0;
    let mut consumed = None;
    let mut object = None;
    while let Some(rest) = text.get(cursor..) {
        let trimmed = rest.trim_start();
        cursor = cursor.checked_add(rest.len().checked_sub(trimmed.len())?)?;
        if trimmed.is_empty() {
            break;
        }
        let mut values = serde_json::Deserializer::from_str(trimmed).into_iter::<Json>();
        let Some(Ok(value)) = values.next() else {
            break;
        };
        cursor = cursor.checked_add(values.byte_offset())?;
        consumed = Some(cursor);
        if matches!(value, Json::Object(_)) {
            object = Some(value);
            break;
        }
    }
    let end = consumed?;
    let tail = text.get(end..).unwrap_or_default().to_owned();
    *could_close = updated_could_close(false, &tail);
    buffer.set(tail);
    object
}
