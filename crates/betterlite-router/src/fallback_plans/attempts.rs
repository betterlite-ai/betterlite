use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::fallback_plans::chains::FallbackTarget;
use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

// The identity of one fallback attempt: a bare model group, or the digest of an entry that overrides request parameters
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttemptKey(String);

// The fallback attempts one request has already made, created on its first hop and carried for the rest of the walk
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttemptedTargets {
    keys: BTreeSet<String>,
}

impl AttemptedTargets {
    #[must_use]
    pub fn from_targets(targets: &[FallbackTarget]) -> Self {
        let mut attempted = Self::default();
        for target in targets {
            attempted.record(target);
        }
        attempted
    }

    #[must_use]
    pub fn contains(&self, target: &FallbackTarget) -> bool {
        let Some(key) = attempt_key(target) else {
            return false;
        };
        self.keys.contains(&key.0)
    }

    pub fn record(&mut self, target: &FallbackTarget) {
        if let Some(key) = attempt_key(target) {
            self.keys.insert(key.0);
        }
    }
}

// A name and the same name given as its only field describe one attempt; anything else is hashed, so a large parameter override is not kept twice
#[must_use]
pub fn attempt_key(target: &FallbackTarget) -> Option<AttemptKey> {
    match target {
        FallbackTarget::Model(model) => Some(AttemptKey(model.clone())),
        FallbackTarget::Parameters(parameters) => {
            if parameters.parameters.is_empty() {
                return Some(AttemptKey(parameters.model.clone()));
            }
            let value = serde_json::to_value(parameters).ok()?;
            let mut canonical = String::new();
            canonical_json(&value, &mut canonical)?;
            Some(AttemptKey(hex::encode(Sha256::digest(
                canonical.as_bytes(),
            ))))
        }
    }
}

// Whether a resolved chain still holds an entry this request has not tried
#[must_use]
pub fn has_unattempted_targets(
    chain: Option<&[&FallbackTarget]>,
    attempted: &AttemptedTargets,
) -> bool {
    let Some(targets) = chain else {
        return false;
    };
    targets.iter().any(|target| !attempted.contains(target))
}

// LiteLLM hashes json.dumps(sort_keys=True), so two entries that differ only in key order are one attempt
fn canonical_json(value: &JsonValue, out: &mut String) -> Option<()> {
    match value {
        JsonValue::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_json(item, out)?;
            }
            out.push(']');
        }
        JsonValue::Object(fields) => {
            let sorted: BTreeMap<&String, &JsonValue> = fields.iter().collect();
            out.push('{');
            for (index, (key, item)) in sorted.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).ok()?);
                out.push(':');
                canonical_json(item, out)?;
            }
            out.push('}');
        }
        scalar @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)) => out.push_str(&serde_json::to_string(scalar).ok()?),
    }
    Some(())
}
