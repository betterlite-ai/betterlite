pub const TOOL_NAME_MAX_LEN: usize = 128;

#[must_use]
pub fn sanitize_openapi_tool_name(raw_name: &str) -> String {
    if raw_name.is_empty() {
        return String::new();
    }
    let mut sanitized = String::with_capacity(raw_name.len());
    for letter in raw_name.chars().take(TOOL_NAME_MAX_LEN) {
        if letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-') {
            sanitized.push(letter.to_ascii_lowercase());
            continue;
        }
        sanitized.push('_');
    }
    sanitized
}
