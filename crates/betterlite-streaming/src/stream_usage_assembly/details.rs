use std::borrow::Cow;

use super::wire::{CacheCreationTokenDetails, PromptTokensDetails};

// The 1h/5m breakdown only rides on message_start, so a later flat prompt-detail record must not drop it.
#[must_use]
pub fn capture_cache_breakdown(
    prompt_tokens_details: Option<&PromptTokensDetails>,
    current: Option<CacheCreationTokenDetails>,
) -> Option<CacheCreationTokenDetails> {
    prompt_tokens_details
        .and_then(|details| details.cache_creation_token_details.clone())
        .or(current)
}

// A details record that already carries a breakdown keeps it; the caller gets its own value back untouched.
#[must_use]
pub fn attach_cache_breakdown<'details>(
    prompt_tokens_details: Option<&'details PromptTokensDetails>,
    cache_creation_token_details: Option<&CacheCreationTokenDetails>,
) -> Option<Cow<'details, PromptTokensDetails>> {
    let details = prompt_tokens_details?;
    let Some(cache) = cache_creation_token_details else {
        return Some(Cow::Borrowed(details));
    };
    if details.cache_creation_token_details.is_some() {
        return Some(Cow::Borrowed(details));
    }
    let mut updated = details.clone();
    updated.cache_creation_token_details = Some(cache.clone());
    Some(Cow::Owned(updated))
}

#[must_use]
pub fn apply_grounding_request_counts(
    prompt_tokens_details: Option<&PromptTokensDetails>,
    web_search_requests: Option<i64>,
    google_maps_grounding_requests: Option<i64>,
) -> Option<Cow<'_, PromptTokensDetails>> {
    if web_search_requests.is_none() && google_maps_grounding_requests.is_none() {
        return prompt_tokens_details.map(Cow::Borrowed);
    }
    let mut counted = prompt_tokens_details.cloned().unwrap_or_default();
    if let Some(requests) = web_search_requests {
        counted.web_search_requests = Some(requests);
    }
    if let Some(requests) = google_maps_grounding_requests {
        counted.google_maps_grounding_requests = Some(requests);
    }
    Some(Cow::Owned(counted))
}
