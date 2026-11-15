use sha2::{Digest as _, Sha256};

use crate::credentials::ANTHROPIC_VERSION;

// LiteLLM's optional version argument defaults to 2023-06-01 at every call site it owns
#[must_use]
pub fn cache_scope(
    caller_key_hash: &str,
    deployment_id: &str,
    provider_key: &str,
    model: &str,
) -> String {
    let scope = (
        caller_key_hash,
        deployment_id,
        provider_key,
        model,
        ANTHROPIC_VERSION,
    );
    // every scope field is a string, so the tuple cannot fail to serialize
    let encoded = serde_json::to_vec(&scope).unwrap_or_default();
    hex::encode(Sha256::digest(encoded))
}
