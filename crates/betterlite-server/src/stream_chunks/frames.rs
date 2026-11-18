use super::SSE_FRAME_DELIMITERS;

#[must_use]
pub fn sse_data_frame(payload: &str) -> String {
    let capacity = "data: "
        .len()
        .saturating_add(payload.len())
        .saturating_add("\n\n".len());
    let mut frame = String::with_capacity(capacity);
    frame.push_str("data: ");
    frame.push_str(payload);
    frame.push_str("\n\n");
    frame
}

#[must_use]
pub fn ends_with_frame_delimiter(text: &str) -> bool {
    for delimiter in SSE_FRAME_DELIMITERS {
        if text.ends_with(delimiter) {
            return true;
        }
    }
    false
}

#[must_use]
pub fn pop_sse_frame(buffer: &str) -> (Option<String>, String) {
    let earliest = SSE_FRAME_DELIMITERS
        .iter()
        .filter_map(|delimiter| {
            buffer
                .find(delimiter)
                .map(|position| (position, delimiter.len()))
        })
        .min_by_key(|(position, _length)| *position);
    let Some((position, length)) = earliest else {
        return (None, buffer.to_owned());
    };
    let frame_end = position.saturating_add(length);
    let Some((frame, rest)) = buffer.split_at_checked(frame_end) else {
        return (None, buffer.to_owned());
    };
    (Some(frame.to_owned()), rest.to_owned())
}
