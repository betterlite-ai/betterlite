use http::HeaderMap;
use http::header::AUTHORIZATION;

mod tests;

pub(super) const LITELLM_KEY_HEADER: &str = "x-litellm-api-key";
const BEARER_PREFIX_CHARS: usize = 7;

// str.strip() and str.isspace() also count U+001C to U+001F, which char::is_whitespace leaves out
const fn is_python_space(letter: char) -> bool {
    match letter {
        '\u{1c}'..='\u{1f}' => true,
        _ => letter.is_whitespace(),
    }
}

// Starlette decodes header bytes as Latin-1, one char per byte, then strips and drops a bearer prefix
#[must_use]
pub fn litellm_key_from_request(headers: &HeaderMap) -> Option<String> {
    for name in [LITELLM_KEY_HEADER, AUTHORIZATION.as_str()] {
        let Some(value) = headers.get(name) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let text: String = value.as_bytes().iter().copied().map(char::from).collect();
        let stripped = text.trim_matches(is_python_space);
        let token = if stripped.to_lowercase().starts_with("bearer ") {
            let remainder: String = stripped.chars().skip(BEARER_PREFIX_CHARS).collect();
            remainder.trim_matches(is_python_space)
        } else {
            stripped
        };
        if !token.is_empty() {
            return Some(token.to_owned());
        }
    }
    None
}
