#[must_use]
pub fn is_same_origin_return_path(return_to: &str) -> bool {
    if !return_to.starts_with('/') || return_to.starts_with("//") || return_to.contains('\\') {
        return false;
    }
    !return_to
        .chars()
        .any(|letter| letter.is_ascii_control() || letter == ' ')
}
