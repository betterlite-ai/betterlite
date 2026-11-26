use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;

const ANTHROPIC_BETA: &str = "anthropic-beta";
const ALLOWED_VERTEX_HEADERS: [&str; 2] = [ANTHROPIC_BETA, "content-type"];
const COUNT_TOKENS: &str = "count-tokens";

// LiteLLM's get_vertex_ai_allowed_incoming_headers over the request's cached headers, which are never mutated
#[must_use]
pub fn allowed_vertex_headers(headers: &HeaderMap) -> IndexMap<String, SecretString> {
    let mut allowed = IndexMap::with_capacity(ALLOWED_VERTEX_HEADERS.len());
    for name in ALLOWED_VERTEX_HEADERS {
        let Some(value) = headers.get(name) else {
            continue;
        };
        allowed.insert(name.to_owned(), SecretString::from(header_text(value)));
    }
    allowed
}

// Starlette decodes header bytes as latin-1
fn header_text(value: &HeaderValue) -> String {
    value
        .as_bytes()
        .iter()
        .copied()
        .map(char::from)
        .collect::<String>()
}

// LiteLLM's _is_vertex_anthropic_count_tokens_route: the segment after the last slash, before the first colon
#[must_use]
pub fn is_vertex_count_tokens_route(endpoint: &str) -> bool {
    let last = endpoint
        .rsplit_once('/')
        .map_or(endpoint, |(_head, tail)| tail);
    last.split_once(':')
        .map_or(last, |(segment, _tail)| segment)
        == COUNT_TOKENS
}

// LiteLLM's _upstream_headers_for_vertex_route: only the count-tokens route drops anthropic-beta
#[must_use]
pub fn vertex_upstream_headers(
    endpoint: &str,
    headers: &IndexMap<String, SecretString>,
) -> IndexMap<String, SecretString> {
    if !is_vertex_count_tokens_route(endpoint) {
        return headers.clone();
    }
    headers
        .iter()
        .filter(|(name, _value)| !name.eq_ignore_ascii_case(ANTHROPIC_BETA))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
