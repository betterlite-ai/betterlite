use betterlite_models::decisions::Json;

use crate::gemini_metadata::extension::MetadataExtension;

#[must_use]
pub fn extract_candidate_metadata(candidate: &Json) -> MetadataExtension {
    let mut extension = MetadataExtension::default();
    if let Some(grounding) = present(candidate, "groundingMetadata") {
        if let Json::List(entries) = grounding {
            extension.grounding_metadata.extend(entries.iter().cloned());
        } else {
            extension.grounding_metadata.push(grounding.clone());
        }
    }
    if let Some(safety) = present(candidate, "safetyRatings") {
        extension.safety_ratings.push(safety.clone());
        extension.safety_results.push(safety.clone());
    }
    if let Some(citation) = present(candidate, "citationMetadata") {
        extension.citation_metadata.push(citation.clone());
    }
    if let Some(url_context) = present(candidate, "urlContextMetadata") {
        extension.url_context_metadata.push(url_context.clone());
    }
    extension
}

// LiteLLM tests key presence, and an explicit null takes the declared default like an absent key
fn present<'candidate>(candidate: &'candidate Json, name: &str) -> Option<&'candidate Json> {
    let Json::Object(fields) = candidate else {
        return None;
    };
    match fields.get(name) {
        Some(Json::Null) | None => None,
        Some(value) => Some(value),
    }
}
