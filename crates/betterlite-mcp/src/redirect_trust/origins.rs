use crate::oauth_resource::base_url::strip_default_port;

// LiteLLM's _parse_trusted_redirect_origins: comma-separated hosts, lowercased, scheme and path dropped.
#[must_use]
pub fn parse_trusted_redirect_origins(text: &str) -> Vec<String> {
    let raw = text.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    let mut entries: Vec<String> = Vec::new();
    for token in raw.split(',') {
        let mut entry = token.trim().to_lowercase();
        if entry.is_empty() {
            continue;
        }
        if let Some((_, after_scheme)) = entry.split_once("://") {
            entry = after_scheme.to_owned();
        }
        if let Some((before_path, _)) = entry.split_once('/') {
            entry = before_path.to_owned();
        }
        if entry.is_empty() {
            continue;
        }
        if !entry.starts_with("*.") {
            entry = strip_default_port("https", &entry);
        }
        if !entry.is_empty() {
            entries.push(entry);
        }
    }
    entries
}
