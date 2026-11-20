use betterlite_bedrock_guardrails::constants::BEDROCK_PII_LOCATION_KEYS;
use serde_json::Number;

use crate::bedrock_records::{ChecksPiiEntry, ChecksResponse, ChecksScoreResult};

// LiteLLM's _sanitize_invoke_checks_response_for_logging: the logged copy loses every PII offset
#[must_use]
pub fn for_logging(response: &ChecksResponse) -> ChecksResponse {
    let mut sanitized = response.clone();
    let sensitive = sanitized
        .results
        .as_mut()
        .and_then(|results| results.sensitive_information.as_mut());
    if let Some(pii) = sensitive
        && let Some(entries) = pii.results.as_mut()
    {
        for entry in entries {
            strip_location(entry);
        }
    }
    as_floats(&mut sanitized);
    sanitized
}

fn strip_location(entry: &mut ChecksPiiEntry) {
    entry.message_index = None;
    entry.content_index = None;
    entry.begin_offset = None;
    entry.end_offset = None;
    for key in BEDROCK_PII_LOCATION_KEYS {
        drop(entry.extra.remove(key));
    }
}

// the validated TypedDict declares every score a float, so the logged copy carries 1.0 where the body said 1
fn as_floats(response: &mut ChecksResponse) {
    let Some(results) = response.results.as_mut() else {
        return;
    };
    if let Some(result) = results.content_filter.as_mut() {
        float_scores(result);
    }
    if let Some(result) = results.prompt_attack.as_mut() {
        float_scores(result);
    }
    let entries = results
        .sensitive_information
        .as_mut()
        .and_then(|sensitive| sensitive.results.as_mut());
    if let Some(pii_entries) = entries {
        for entry in pii_entries {
            entry.confidence_score = entry
                .confidence_score
                .as_ref()
                .and_then(Number::as_f64)
                .and_then(Number::from_f64);
        }
    }
}

fn float_scores(result: &mut ChecksScoreResult) {
    let Some(entries) = result.results.as_mut() else {
        return;
    };
    for entry in entries {
        entry.severity_score = entry
            .severity_score
            .as_ref()
            .and_then(Number::as_f64)
            .and_then(Number::from_f64);
    }
}
