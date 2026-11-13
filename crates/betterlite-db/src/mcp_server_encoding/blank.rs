#[must_use]
pub fn blank_to_none(value: Option<&str>) -> Option<&str> {
    let text = value?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}
