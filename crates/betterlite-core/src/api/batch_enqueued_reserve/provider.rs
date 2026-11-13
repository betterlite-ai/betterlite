use crate::api::file_routing_ids::legacy::unified_batch_id;
use crate::api::file_routing_ids::routing::get_original_file_id;
use crate::api::managed_resource_ids::is_base64_encoded_unified_id;

const MANAGED_FILE_ID_PREFIX: &str = "litellm_proxy";
const BATCH_ID_MARKER: &str = "llm_batch_id";
const GENERIC_RESPONSE_ID_MARKER: &str = "generic_response_id";

#[must_use]
pub fn canonical_provider_batch_id(batch_id: &str) -> String {
    let Some(decoded) = is_base64_encoded_unified_id(batch_id, MANAGED_FILE_ID_PREFIX) else {
        return get_original_file_id(batch_id);
    };
    if !decoded.contains(BATCH_ID_MARKER) && !decoded.contains(GENERIC_RESPONSE_ID_MARKER) {
        return decoded;
    }
    // the source raises IndexError when the marker text has no colon; the input comes back unchanged instead
    unified_batch_id(&decoded).unwrap_or_else(|_error| batch_id.to_owned())
}
