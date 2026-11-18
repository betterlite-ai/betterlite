use std::collections::HashSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};

use super::reply::{Annotation, GroundingEnvelope};
use super::{RESPONSE_COST_HEADER, SNIPPET_FALLBACK_LENGTH};

#[must_use]
pub fn failed_detail(envelope: &GroundingEnvelope) -> &str {
    let message = envelope
        .error
        .as_ref()
        .and_then(|error| error.message.as_deref());
    match message {
        Some(message) if !message.is_empty() => message,
        Some(_) | None => "the grounded search failed",
    }
}

#[must_use]
pub fn incomplete_reason(envelope: &GroundingEnvelope) -> &str {
    let reason = envelope
        .incomplete_details
        .as_ref()
        .and_then(|details| details.reason.as_deref());
    match reason {
        Some(reason) if !reason.is_empty() => reason,
        Some(_) | None => "unknown reason",
    }
}

#[must_use]
pub fn requested_max_results(optional_params: Option<&Object>) -> Option<i64> {
    optional_params
        .and_then(|params| params.get("max_results"))
        .and_then(JsonValue::as_i64)
        .filter(|count| *count > 0)
}

#[must_use]
pub fn capped(results: Vec<SearchResult>, max_results: Option<i64>) -> Vec<SearchResult> {
    let Some(max_results) = max_results else {
        return results;
    };
    let Ok(limit) = usize::try_from(max_results) else {
        return results;
    };
    results.into_iter().take(limit).collect()
}

#[must_use]
pub fn citation_results(envelope: &GroundingEnvelope) -> Vec<SearchResult> {
    let mut cited = Vec::new();
    for item in &envelope.output {
        if item.kind != "message" {
            continue;
        }
        for part in &item.content {
            if part.kind != "output_text" {
                continue;
            }
            for annotation in &part.annotations {
                if annotation.kind != "url_citation" {
                    continue;
                }
                let Some(url) = annotation.url.as_deref().filter(|url| !url.is_empty()) else {
                    continue;
                };
                cited.push(SearchResult {
                    title: annotation.title.clone().unwrap_or_default(),
                    url: url.to_owned(),
                    snippet: snippet(&part.text, annotation),
                    date: None,
                    last_updated: None,
                    extra: Object::new(),
                });
            }
        }
    }
    first_by_url(cited)
}

// the first result for a URL wins and the answer order decides the output order
fn first_by_url(cited: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut seen = HashSet::new();
    cited
        .into_iter()
        .filter(|result| seen.insert(result.url.clone()))
        .collect()
}

#[must_use]
pub fn snippet(text: &str, annotation: &Annotation) -> String {
    let chars: Vec<char> = text.chars().collect();
    let marker_start = annotation
        .start_index
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index <= chars.len())
        .unwrap_or(chars.len());
    let prefix: String = chars.iter().take(marker_start).copied().collect();
    let claim = prefix.rsplit('\n').next().unwrap_or_default().trim();
    if !claim.is_empty() {
        let claim_chars: Vec<char> = claim.chars().collect();
        let start = claim_chars.len().saturating_sub(SNIPPET_FALLBACK_LENGTH);
        return claim_chars.iter().skip(start).copied().collect();
    }
    chars
        .iter()
        .take(SNIPPET_FALLBACK_LENGTH)
        .copied()
        .collect()
}

#[must_use]
pub fn priced(results: Vec<SearchResult>, connection_mode: bool) -> SearchResponse {
    let mut response = SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    };
    if connection_mode {
        return response;
    }
    let mut additional = Object::new();
    additional.insert(String::from(RESPONSE_COST_HEADER), JsonValue::from(0.0_f64));
    let mut hidden = Object::new();
    hidden.insert(
        String::from("additional_headers"),
        JsonValue::Object(additional),
    );
    response.hidden_params = hidden;
    response
}
