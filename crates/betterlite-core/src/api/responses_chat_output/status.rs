use std::collections::BTreeMap;

use betterlite_openai::api::responses_objects::sdk::response::{
    IncompleteDetails, IncompleteReason,
};

#[must_use]
pub fn finish_status(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length" | "content_filter" | "refusal") => "incomplete",
        _ => "completed",
    }
}

#[must_use]
pub fn incomplete_details(
    reason: Option<&str>,
    existing: Option<IncompleteDetails>,
) -> Option<IncompleteDetails> {
    if existing.is_some() {
        return existing;
    }
    match reason {
        Some("length") => Some(IncompleteDetails {
            reason: Some(IncompleteReason::MaxOutputTokens),
            extra: BTreeMap::new(),
        }),
        Some("content_filter" | "refusal") => Some(IncompleteDetails {
            reason: Some(IncompleteReason::ContentFilter),
            extra: BTreeMap::new(),
        }),
        _ => None,
    }
}
