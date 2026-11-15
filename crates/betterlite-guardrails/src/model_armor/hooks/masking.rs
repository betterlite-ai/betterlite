// LiteLLM masks only when Model Armor returned text that is not empty and differs from what it scanned
pub(super) fn changed_text(content: &str, sanitized: Option<String>) -> Option<String> {
    let text = sanitized?;
    if text.is_empty() || text == content {
        return None;
    }
    Some(text)
}
