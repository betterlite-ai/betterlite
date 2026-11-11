use indexmap::IndexMap;

use crate::decisions::Json;

// the declared ModelResponseStream fields plus usage, which streaming sets on its own
const STREAM_RESPONSE_FIELDS: [&str; 8] = [
    "id",
    "created",
    "model",
    "object",
    "system_fingerprint",
    "choices",
    "provider_specific_fields",
    "usage",
];

pub fn preserve_upstream_attributes(
    model_response: &mut IndexMap<String, Json>,
    original_chunk: &IndexMap<String, Json>,
) {
    for (key, value) in original_chunk {
        if !STREAM_RESPONSE_FIELDS.contains(&key.as_str()) {
            model_response.insert(key.clone(), value.clone());
        }
    }
}
