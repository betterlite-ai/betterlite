use betterlite_models::decisions::Json;

use crate::gemini_metadata::extension::MetadataExtension;

#[must_use]
pub fn extract_candidate_metadata(candidate: &Json) -> MetadataExtension {
    let mut extension = MetadataExtension::default();
    let Json::Object(fields) = candidate else {
        return extension;
    };
    // LiteLLM tests key presence, and an explicit null takes the declared default like an absent key
    let present = |name: &str| fields.get(name).filter(|value| **value != Json::Null);
    if let Some(grounding) = present("groundingMetadata") {
        if let Json::List(entries) = grounding {
            extension.grounding_metadata.extend(entries.iter().cloned());
        } else {
            extension.grounding_metadata.push(grounding.clone());
        }
    }
    if let Some(safety) = present("safetyRatings") {
        extension.safety_ratings.push(safety.clone());
        extension.safety_results.push(safety.clone());
    }
    if let Some(citation) = present("citationMetadata") {
        extension.citation_metadata.push(citation.clone());
    }
    if let Some(url_context) = present("urlContextMetadata") {
        extension.url_context_metadata.push(url_context.clone());
    }
    extension
}
