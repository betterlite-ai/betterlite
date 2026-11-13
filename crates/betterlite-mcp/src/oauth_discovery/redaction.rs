use regex::Regex;

use crate::oauth_resource::resource_identity::redact_mcp_resource_url;

pub const UNPARSABLE_URL: &str = "<unparsable url>";

const URL_PATTERN: &str = r"https?://\S+";
const URL_PLACEHOLDER: &str = "<url>";
const MAX_ERROR_TEXT_CHARS: usize = 200;

#[must_use]
pub fn redacted_origin_list(urls: &[String]) -> String {
    urls.iter()
        .map(|url| redact_mcp_resource_url(Some(url)).unwrap_or_else(|| UNPARSABLE_URL.to_owned()))
        .collect::<Vec<String>>()
        .join(", ")
}

#[must_use]
pub fn sanitized_error_text(text: &str) -> String {
    let Ok(pattern) = Regex::new(URL_PATTERN) else {
        return String::new();
    };
    pattern
        .replace_all(text, URL_PLACEHOLDER)
        .chars()
        .take(MAX_ERROR_TEXT_CHARS)
        .collect()
}
