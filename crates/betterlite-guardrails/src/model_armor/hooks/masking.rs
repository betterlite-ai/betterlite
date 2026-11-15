// LiteLLM masks only when Model Armor returned text that is not empty and differs from what it scanned
pub(super) fn changed_text(content: &str, sanitized: Option<String>) -> Option<String> {
    let sanitized = sanitized?;
    if sanitized.is_empty() || sanitized == content {
        return None;
    }
    Some(sanitized)
}
