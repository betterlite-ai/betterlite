use regex::Regex;

use super::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};

// Gemini 2.0 and later accept responseJsonSchema; this is the source's own version check, not the URL one
const GEMINI_2_PLUS_PATTERN: &str = r"gemini-(?:[2-9]|[1-9]\d+)(?:\.|\-)";

pub fn get_supports_response_schema(
    model: &str,
    provider: VertexCapabilityProvider,
    lookup: &dyn ModelCapabilityLookup,
) -> Result<bool, CapabilityLookupError> {
    lookup.supports(
        CapabilityKind::ResponseSchema,
        model,
        provider.lookup_name(),
    )
}

#[must_use]
pub fn supports_response_json_schema(model: &str) -> bool {
    let Ok(pattern) = Regex::new(GEMINI_2_PLUS_PATTERN) else {
        return false;
    };
    pattern.is_match(&model.to_lowercase())
}
