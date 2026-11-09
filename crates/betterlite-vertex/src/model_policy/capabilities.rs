use regex::Regex;
use thiserror::Error;

// Gemini 2.0 and later accept responseJsonSchema; this is the source's own version check, not the URL one
const GEMINI_2_PLUS_PATTERN: &str = r"gemini-(?:[2-9]|[1-9]\d+)(?:\.|\-)";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VertexCapabilityProvider {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct CapabilityLookupError {
    pub message: String,
}

pub trait ModelCapabilityLookup {
    fn supports_system_messages(
        &self,
        model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityLookupError>;

    fn supports_response_schema(
        &self,
        model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityLookupError>;
}

#[must_use]
pub fn get_supports_system_message(
    model: &str,
    provider: VertexCapabilityProvider,
    lookup: &dyn ModelCapabilityLookup,
) -> bool {
    let lookup_result = match provider {
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            lookup.supports_system_messages(model, "vertex_ai")
        }
        VertexCapabilityProvider::Gemini => lookup.supports_system_messages(model, "gemini"),
    };
    match lookup_result {
        Ok(supported) => supported || model.contains("gemini/"),
        Err(error) => {
            tracing::warn!(
                "Unable to identify if system message supported. Defaulting to 'False'. Received error message - {error}\nAdd it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json"
            );
            false
        }
    }
}

pub fn get_supports_response_schema(
    model: &str,
    provider: VertexCapabilityProvider,
    lookup: &dyn ModelCapabilityLookup,
) -> Result<bool, CapabilityLookupError> {
    match provider {
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            lookup.supports_response_schema(model, "vertex_ai")
        }
        VertexCapabilityProvider::Gemini => lookup.supports_response_schema(model, "gemini"),
    }
}

#[must_use]
pub fn supports_response_json_schema(model: &str) -> bool {
    let Ok(pattern) = Regex::new(GEMINI_2_PLUS_PATTERN) else {
        return false;
    };
    pattern.is_match(&model.to_lowercase())
}
