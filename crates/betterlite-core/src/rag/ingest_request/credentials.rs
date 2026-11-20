use crate::api::request_body::JsonObject;

use super::error::IngestRequestError;

// the fields endpoints.py:497-512 blocks by iterating a Python set, checked here in the listed order so the answer never depends on hash seeds
const BLOCKED_CREDENTIAL_FIELDS: [&str; 14] = [
    "vertex_credentials",
    "vertex_ai_credentials",
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "aws_web_identity_token",
    "aws_role_name",
    "aws_session_name",
    "aws_profile_name",
    "aws_sts_endpoint",
    "aws_external_id",
    "azure_ad_token",
    "api_key",
    "api_base",
];

pub(super) fn check(ingest_options: &JsonObject) -> Result<(), IngestRequestError> {
    let Some(vector_store) = ingest_options
        .get("vector_store")
        .and_then(|value| value.as_object())
    else {
        return Ok(());
    };
    for field in BLOCKED_CREDENTIAL_FIELDS {
        if vector_store.contains_key(field) {
            return Err(IngestRequestError::BlockedCredential { field });
        }
    }
    Ok(())
}
