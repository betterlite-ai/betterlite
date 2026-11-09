use crate::oauth_resource::resource_identity::redact_mcp_resource_url;

pub const UNPARSABLE_URL: &str = "<unparsable url>";

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
    let mut sanitized = String::with_capacity(text.len());
    let mut index = 0;
    while let Some(remaining) = text.get(index..) {
        let Some(character) = remaining.chars().next() else {
            break;
        };
        let marker =
            url_marker_length(remaining).filter(|length| has_text_after(remaining, *length));
        if let Some(length) = marker {
            sanitized.push_str(URL_PLACEHOLDER);
            index = skip_url_text(text, index.saturating_add(length));
            continue;
        }
        sanitized.push(character);
        index = index.saturating_add(character.len_utf8());
    }
    sanitized.chars().take(MAX_ERROR_TEXT_CHARS).collect()
}

fn url_marker_length(remaining: &str) -> Option<usize> {
    if remaining.starts_with("https://") {
        Some("https://".len())
    } else if remaining.starts_with("http://") {
        Some("http://".len())
    } else {
        None
    }
}

fn has_text_after(remaining: &str, length: usize) -> bool {
    remaining
        .get(length..)
        .and_then(|tail| tail.chars().next())
        .is_some_and(|character| !character.is_whitespace())
}

fn skip_url_text(text: &str, start: usize) -> usize {
    let mut index = start;
    while let Some(remaining) = text.get(index..) {
        let Some(character) = remaining.chars().next() else {
            break;
        };
        if character.is_whitespace() {
            break;
        }
        index = index.saturating_add(character.len_utf8());
    }
    index
}
