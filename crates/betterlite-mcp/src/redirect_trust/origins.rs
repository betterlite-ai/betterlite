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
        let entry = token.trim().to_lowercase();
        if entry.is_empty() {
            continue;
        }
        let after_scheme = entry
            .split_once("://")
            .map_or(entry.as_str(), |(_, rest)| rest);
        let before_path = after_scheme
            .split_once('/')
            .map_or(after_scheme, |(host, _)| host);
        if before_path.is_empty() {
            continue;
        }
        let mut netloc = before_path.to_owned();
        if !netloc.starts_with("*.") {
            netloc = strip_default_port("https", &netloc);
        }
        if !netloc.is_empty() {
            entries.push(netloc);
        }
    }
    entries
}
