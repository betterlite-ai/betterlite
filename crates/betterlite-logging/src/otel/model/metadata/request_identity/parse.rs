use super::super::helpers::{flatten_metadata, resolve_provider_model};
use super::super::wire::MetadataError;
use super::super::wire::payload::Payload;
use super::RequestIdentity;
use crate::otel::model::utils::coercion::OtelModelValue;

pub fn from_payload(payload: &Payload) -> Result<RequestIdentity, MetadataError> {
    let metadata = &payload.metadata;
    let raw: OtelModelValue = serde_json::from_value(serde_json::to_value(metadata)?)?;
    Ok(RequestIdentity {
        call_id: payload
            .litellm_call_id
            .clone()
            .or_else(|| payload.id.clone()),
        team_id: metadata
            .user_api_key_team_id
            .clone()
            .or_else(|| metadata.team_id.clone()),
        team_alias: metadata
            .user_api_key_team_alias
            .clone()
            .or_else(|| metadata.team_alias.clone()),
        team_metadata: metadata
            .user_api_key_team_metadata
            .clone()
            .filter(|fields| !fields.is_empty()),
        key_hash: metadata.user_api_key_hash.clone(),
        end_user: payload
            .end_user
            .clone()
            .or_else(|| metadata.user_api_key_end_user_id.clone()),
        provider_model: resolve_provider_model(payload),
        request_route: metadata.user_api_key_request_route.clone(),
        metadata: flatten_metadata(raw.as_object()),
    })
}

mod tests;
