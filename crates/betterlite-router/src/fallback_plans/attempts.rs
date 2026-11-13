use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::fallback_plans::chains::{FallbackTarget, ParameterizedTarget};
use betterlite_config::sections::settings::JsonValue;
use sha2::{Digest as _, Sha256};

// LiteLLM treats a bare name and {"model": name} as one attempt, and hashes any other entry with sort_keys=True
#[must_use]
pub fn attempt_key(target: &FallbackTarget) -> String {
    match target {
        FallbackTarget::Model(model) => model.clone(),
        FallbackTarget::Parameters(parameters) if parameters.parameters.is_empty() => {
            parameters.model.clone()
        }
        FallbackTarget::Parameters(parameters) => {
            hex::encode(Sha256::digest(canonical_entry(parameters).as_bytes()))
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttemptedTargets {
    pub keys: BTreeSet<String>,
}

impl AttemptedTargets {
    #[must_use]
    pub fn from_targets(targets: &[FallbackTarget]) -> Self {
        let mut attempted = Self::default();
        for target in targets {
            attempted.keys.insert(attempt_key(target));
        }
        attempted
    }
}

#[must_use]
pub fn has_unattempted_targets(targets: &[&FallbackTarget], attempted: &AttemptedTargets) -> bool {
    for target in targets {
        if !attempted.keys.contains(&attempt_key(target)) {
            return true;
        }
    }
    false
}

// LiteLLM hashes json.dumps(sort_keys=True), so every object's keys are sorted before hashing
fn canonical_entry(target: &ParameterizedTarget) -> String {
    let mut fields: BTreeMap<&str, String> = BTreeMap::new();
    fields.insert("model", JsonValue::String(target.model.clone()).to_string());
    for (key, value) in &target.parameters {
        fields.insert(key.as_str(), canonical_value(value));
    }
    join_object(fields)
}

fn canonical_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Array(items) => {
            let mut out = String::from("[");
            let mut separator = "";
            for item in items {
                out.push_str(separator);
                separator = ",";
                out.push_str(&canonical_value(item));
            }
            out.push(']');
            out
        }
        JsonValue::Object(fields) => {
            let sorted: BTreeMap<&str, String> = fields
                .iter()
                .map(|(key, field)| (key.as_str(), canonical_value(field)))
                .collect();
            join_object(sorted)
        }
        scalar @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)) => scalar.to_string(),
    }
}

fn join_object(fields: BTreeMap<&str, String>) -> String {
    let mut out = String::from("{");
    let mut separator = "";
    for (key, value) in fields {
        out.push_str(separator);
        separator = ",";
        out.push_str(&JsonValue::String(key.to_owned()).to_string());
        out.push(':');
        out.push_str(&value);
    }
    out.push('}');
    out
}
