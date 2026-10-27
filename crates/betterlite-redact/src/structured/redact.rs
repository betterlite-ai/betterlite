use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{DEFAULT_REDACT_DEPTH, MAX_WORK, REDACTED, SensitiveDataMasker, walk};

#[must_use]
pub fn mask_sensitive_keys(
    data: &Map<String, Value>,
    sensitive_fields: &BTreeSet<String>,
) -> Value {
    let masker = SensitiveDataMasker::new();
    let mut walker = walk::Walker {
        masker: &masker,
        kind: walk::Kind::Exact(sensitive_fields),
        excluded_keys: &[],
        max_depth: DEFAULT_REDACT_DEPTH,
        remaining: MAX_WORK,
    };
    walker
        .mapping(data, 0)
        .unwrap_or_else(|| Value::String(REDACTED.to_owned()))
}

#[must_use]
pub fn redact_credentials_in_payload(data: &Map<String, Value>) -> Value {
    let masker = SensitiveDataMasker::new();
    let mut walker = walk::Walker {
        masker: &masker,
        kind: walk::Kind::Redact,
        excluded_keys: &[],
        max_depth: DEFAULT_REDACT_DEPTH.saturating_add(1),
        remaining: MAX_WORK,
    };
    walker
        .mapping(data, 0)
        .unwrap_or_else(|| Value::String(REDACTED.to_owned()))
}
