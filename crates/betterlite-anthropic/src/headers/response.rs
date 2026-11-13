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
    for (name, value) in headers {
        projected.insert(format!("{PROVIDER_PREFIX}{name}"), latin1(value.as_bytes()));
    }
    for (source, target) in RATE_LIMIT_ALIASES {
        if let Some(value) = headers.get(source) {
            projected.insert(target.to_owned(), latin1(value.as_bytes()));
        }
    }
    projected
}

// httpx hands header bytes to Python as latin-1 text, which never fails
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}
