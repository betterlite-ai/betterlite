use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use super::error::EvalsGetUpdateError;

// LiteLLM's internal keys, kept in its order; a user metadata field with one of these names never reaches the wire
const INTERNAL_METADATA_KEYS: [&str; 30] = [
    "headers",
    "requester_metadata",
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_spend",
    "user_api_key_max_budget",
    "user_api_key_team_id",
    "user_api_key_user_id",
    "user_api_key_org_id",
    "user_api_key_team_alias",
    "user_api_key_end_user_id",
    "user_api_key_user_email",
    "user_api_key_request_route",
    "user_api_key_budget_reset_at",
    "user_api_key_auth_metadata",
    "user_api_key",
    "user_api_end_user_max_budget",
    "user_api_key_auth",
    "litellm_api_version",
    "global_max_parallel_requests",
    "user_api_key_team_max_budget",
    "user_api_key_team_spend",
    "user_api_key_model_max_budget",
    "user_api_key_user_spend",
    "user_api_key_user_max_budget",
    "user_api_key_metadata",
    "endpoint",
    "litellm_parent_otel_span",
    "requester_ip_address",
    "user_agent",
];

// the caller's metadata without LiteLLM's keys, and no metadata field at all when nothing is left
pub fn filtered_metadata<T>(
    metadata: Option<&IndexMap<String, Json>>,
) -> Result<Option<T>, EvalsGetUpdateError>
where
    T: DeserializeOwned,
{
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let mut kept = IndexMap::new();
    for (name, value) in metadata {
        if INTERNAL_METADATA_KEYS.contains(&name.as_str()) {
            continue;
        }
        kept.insert(name.clone(), value.clone());
    }
    if kept.is_empty() {
        return Ok(None);
    }
    serde_json::to_value(&kept)
        .and_then(serde_json::from_value::<T>)
        .map(Some)
        .map_err(|_source| EvalsGetUpdateError::FieldType {
            name: "metadata".to_owned(),
        })
}
