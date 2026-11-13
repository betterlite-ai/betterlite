use http::HeaderMap;
use indexmap::IndexMap;

const PROVIDER_PREFIX: &str = "llm_provider-";
const RATE_LIMIT_ALIASES: [(&str, &str); 4] = [
    (
        "anthropic-ratelimit-requests-limit",
        "x-ratelimit-limit-requests",
    ),
    (
        "anthropic-ratelimit-requests-remaining",
        "x-ratelimit-remaining-requests",
    ),
    (
        "anthropic-ratelimit-tokens-limit",
        "x-ratelimit-limit-tokens",
    ),
    (
        "anthropic-ratelimit-tokens-remaining",
        "x-ratelimit-remaining-tokens",
    ),
];

#[must_use]
pub fn process_anthropic_headers(headers: &HeaderMap) -> IndexMap<String, String> {
    let mut projected = IndexMap::new();
    for name in headers.keys() {
        if let Some(value) = joined(headers, name.as_str()) {
            projected.insert(format!("{PROVIDER_PREFIX}{name}"), value);
        }
    }
    for (source, target) in RATE_LIMIT_ALIASES {
        if let Some(value) = joined(headers, source) {
            projected.insert(target.to_owned(), value);
        }
    }
    projected
}

// httpx reads a repeated header as one value joined with a comma and a space, and decodes bytes as latin-1
fn joined(headers: &HeaderMap, name: &str) -> Option<String> {
    let values: Vec<String> = headers
        .get_all(name)
        .iter()
        .map(|value| value.as_bytes().iter().copied().map(char::from).collect())
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}
