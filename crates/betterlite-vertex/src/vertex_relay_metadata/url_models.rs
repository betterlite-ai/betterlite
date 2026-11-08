use regex::Regex;
use url::Url;

use super::types::{CustomLlmProvider, VertexPublisher};

const MODEL_PATH_PATTERN: &str = "/models/([^:]+)";
const UNKNOWN_MODEL: &str = "unknown";

#[must_use]
pub fn extract_model_from_url(url: &str) -> String {
    let Ok(pattern) = Regex::new(MODEL_PATH_PATTERN) else {
        return UNKNOWN_MODEL.to_owned();
    };
    let Some(captures) = pattern.captures(url) else {
        return UNKNOWN_MODEL.to_owned();
    };
    captures.get(1).map_or_else(
        || UNKNOWN_MODEL.to_owned(),
        |model| model.as_str().to_owned(),
    )
}

#[must_use]
pub fn publisher_from_url(url: &str) -> Option<VertexPublisher> {
    if url.contains("/publishers/mistralai/") {
        Some(VertexPublisher::MistralAi)
    } else if url.contains("/publishers/anthropic/") {
        Some(VertexPublisher::Anthropic)
    } else if url.contains("/publishers/ai21/") {
        Some(VertexPublisher::Ai21)
    } else if url.contains("/endpoints/openapi/") {
        Some(VertexPublisher::OpenApi)
    } else {
        None
    }
}

#[must_use]
pub fn custom_provider_from_url(url: &str) -> CustomLlmProvider {
    let Ok(parsed) = Url::parse(url) else {
        return CustomLlmProvider::VertexAi;
    };
    match parsed.host_str() {
        Some(host) if host.ends_with("generativelanguage.googleapis.com") => {
            CustomLlmProvider::Gemini
        }
        _ => CustomLlmProvider::VertexAi,
    }
}
