use crate::openai_classification::{is_openai_compatible_host, parse_route};

mod tests;

#[must_use]
// ast-grep-ignore: no-long-names
pub fn is_openai_chat_completions_route(url_route: &str) -> bool {
    let parsed = parse_route(url_route);
    is_openai_compatible_host(parsed.hostname.as_deref())
        && parsed.path.contains("/v1/chat/completions")
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn is_openai_image_generation_route(url_route: &str) -> bool {
    let parsed = parse_route(url_route);
    is_openai_compatible_host(parsed.hostname.as_deref())
        && parsed.path.contains("/v1/images/generations")
}

#[must_use]
pub fn is_openai_image_editing_route(url_route: &str) -> bool {
    let parsed = parse_route(url_route);
    is_openai_compatible_host(parsed.hostname.as_deref())
        && parsed.path.contains("/v1/images/edits")
}

#[must_use]
pub fn is_openai_responses_route(url_route: &str) -> bool {
    let parsed = parse_route(url_route);
    // "/v1/responses" contains "/responses", so one test covers both of the source's variants.
    is_openai_compatible_host(parsed.hostname.as_deref()) && parsed.path.contains("/responses")
}

#[must_use]
pub fn is_openai_embeddings_route(url_route: &str) -> bool {
    let parsed = parse_route(url_route);
    is_openai_compatible_host(parsed.hostname.as_deref()) && parsed.path.contains("/v1/embeddings")
}
