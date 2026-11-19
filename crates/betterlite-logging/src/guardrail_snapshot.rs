use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};

use crate::callback_logger::event::GuardrailRequestSnapshot;

const GUARDRAIL_INFORMATION_KEY: &str = "standard_logging_guardrail_information";
const GUARDRAIL_MODE_KEY: &str = "guardrail_mode";

// the hooks that never rewrite the outbound request; any other mode, including one added later, counts as mutating
const NON_MUTATING_GUARDRAIL_MODES: [&str; 6] = [
    "during_call",
    "post_call",
    "logging_only",
    "during_mcp_call",
    "post_mcp_call",
    "realtime_input_transcription",
];

// a logging_only entry cannot rewrite the request, but it records a guardrail run, so replay must see it change the fingerprint
const REPLAY_SAFE_GUARDRAIL_MODES: [&str; 5] = [
    "during_call",
    "post_call",
    "during_mcp_call",
    "post_mcp_call",
    "realtime_input_transcription",
];

#[must_use]
// ast-grep-ignore: no-long-names
pub fn request_mutating_guardrail_ran(metadata: &IndexMap<String, Json>) -> bool {
    guardrail_entries(metadata)
        .any(|entry| !guardrail_is_non_mutating(entry, &NON_MUTATING_GUARDRAIL_MODES))
}

#[must_use]
pub fn request_guardrail_fingerprint(metadata: &IndexMap<String, Json>) -> Option<String> {
    let relevant: Vec<Json> = guardrail_entries(metadata)
        .filter(|entry| !guardrail_is_non_mutating(entry, &REPLAY_SAFE_GUARDRAIL_MODES))
        .map(|entry| Json::Object(canonical_object(entry)))
        .collect();
    let serialized = serde_json::to_string(&relevant).ok()?;
    Some(hex::encode(Sha256::digest(serialized.as_bytes())))
}

impl GuardrailRequestSnapshot {
    #[must_use]
    pub fn capture(
        body: &IndexMap<String, Json>,
        metadata: &IndexMap<String, Json>,
    ) -> Option<Self> {
        // ast-grep-ignore: no-long-names
        if !request_mutating_guardrail_ran(metadata) {
            return None;
        }
        let fingerprint = request_guardrail_fingerprint(metadata)?;
        Some(Self {
            body: body.clone(),
            fingerprint,
        })
    }
}

fn guardrail_entries(
    metadata: &IndexMap<String, Json>,
) -> impl Iterator<Item = &IndexMap<String, Json>> {
    let items = match metadata.get(GUARDRAIL_INFORMATION_KEY) {
        Some(Json::List(items)) => items.as_slice(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => &[],
    };
    items.iter().filter_map(|entry| match entry {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    })
}

fn guardrail_is_non_mutating(entry: &IndexMap<String, Json>, allowed: &[&str]) -> bool {
    let Some(mode) = entry.get(GUARDRAIL_MODE_KEY) else {
        return false;
    };
    match mode {
        Json::List(modes) => modes.iter().all(|item| mode_is_allowed(item, allowed)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            mode_is_allowed(mode, allowed)
        }
    }
}

fn mode_is_allowed(mode: &Json, allowed: &[&str]) -> bool {
    match mode {
        Json::Text(text) => allowed.contains(&text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) | Json::List(_) => false,
    }
}

fn canonical_object(fields: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut ordered: BTreeMap<&str, Json> = BTreeMap::new();
    for (name, value) in fields {
        ordered.insert(name.as_str(), canonical_json(value));
    }
    ordered
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn canonical_json(value: &Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(canonical_object(fields)),
        Json::List(items) => Json::List(items.iter().map(canonical_json).collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
    }
}

mod tests;
