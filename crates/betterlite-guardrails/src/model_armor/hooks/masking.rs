pub(super) fn changed_text(content: &str, sanitized: Option<String>) -> Option<String> {
    // LiteLLM masks only a non-empty text that differs from what it scanned
    let text = sanitized?;
    if text.is_empty() || text == content {
        return None;
    }
    Some(text)
}
