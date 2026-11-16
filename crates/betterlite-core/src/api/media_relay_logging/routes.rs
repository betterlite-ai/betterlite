use super::fal::PROVIDER;
use crate::openai_classification::parse_route;

const DEEPGRAM_SEGMENT: &str = "/deepgram/";
const LISTEN_SUFFIX: &str = "/listen";

// The proxy route carries the deepgram endpoint before the listen path.
#[must_use]
pub fn is_deepgram_listen_route(url_route: &str) -> bool {
    let path = parse_route(url_route).path;
    if !path.contains(DEEPGRAM_SEGMENT) {
        return false;
    }
    path.ends_with(LISTEN_SUFFIX)
}

// The route is in the signature so the dispatcher mirrors the source; only the provider decides here.
#[must_use]
pub fn is_fal_ai_route(_url_route: &str, custom_llm_provider: Option<&str>) -> bool {
    match custom_llm_provider {
        Some(provider) => provider == PROVIDER,
        None => false,
    }
}
