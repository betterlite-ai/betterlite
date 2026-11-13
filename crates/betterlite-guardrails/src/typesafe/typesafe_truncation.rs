use crate::typesafe::ELISION_MARKER;

#[must_use]
pub fn truncate_for_state(text: &str, max_chars: usize) -> String {
    let length = text.chars().count();
    if length <= max_chars {
        return text.to_owned();
    }
    let marker_length = ELISION_MARKER.chars().count();
    if max_chars <= marker_length {
        return text.chars().take(max_chars).collect();
    }
    let budget = max_chars.saturating_sub(marker_length);
    let head = budget.checked_div(2).unwrap_or(0);
    let tail = budget.saturating_sub(head);
    let mut truncated: String = text.chars().take(head).collect();
    truncated.push_str(ELISION_MARKER);
    truncated.extend(text.chars().skip(length.saturating_sub(tail)));
    truncated
}

#[path = "typesafe_truncation/tests.rs"]
mod tests;
