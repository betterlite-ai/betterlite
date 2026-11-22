use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::SCAN_METADATA_KEYS;
use super::record_json::dumps::{Rendering, dumps};
use super::record_json::text::RecordText;
use super::record_json::{RecordJson, RecordObject};

// Order-insensitive, and an absent key projects to None while a null one projects to Some("null"), so adding or dropping a null still reads as a change
#[must_use]
pub fn fingerprint(body: &RecordObject, keys: &BTreeSet<&RecordText>) -> Vec<Option<String>> {
    keys.iter()
        .map(|key| {
            body.get(*key)
                .map(|value| dumps(value, Rendering::Canonical))
        })
        .collect()
}

// Only what guardrail dispatch reads: the parent OTel span would put one guardrail span per record on the upload's trace
#[must_use]
pub fn build_scan_metadata(request_metadata: &IndexMap<String, Json>) -> RecordObject {
    request_metadata
        .iter()
        .filter(|(key, _)| SCAN_METADATA_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (RecordText::from(key.as_str()), RecordJson::from(value)))
        .collect()
}
