use super::super::common::{field, object, string, team_metadata};
use super::super::helpers::{flatten_metadata, resolve_provider_model};
use super::RequestIdentity;
use crate::otel::model::utils::coercion::OtelModelValue;

#[must_use]
pub fn from_payload(payload: &OtelModelValue) -> RequestIdentity {
    let metadata = object(field(payload, "metadata"));
    RequestIdentity {
        call_id: string(field(payload, "litellm_call_id")).or_else(|| string(field(payload, "id"))),
        team_id: string(metadata.and_then(|values| values.get("user_api_key_team_id")))
            .or_else(|| string(metadata.and_then(|values| values.get("team_id")))),
        team_alias: string(metadata.and_then(|values| values.get("user_api_key_team_alias")))
            .or_else(|| string(metadata.and_then(|values| values.get("team_alias")))),
        team_metadata: team_metadata(
            metadata.and_then(|values| values.get("user_api_key_team_metadata")),
        ),
        key_hash: string(metadata.and_then(|values| values.get("user_api_key_hash"))),
        end_user: string(field(payload, "end_user"))
            .or_else(|| string(metadata.and_then(|values| values.get("user_api_key_end_user_id")))),
        provider_model: resolve_provider_model(payload),
        request_route: metadata
            .and_then(|values| values.get("user_api_key_request_route"))
            .and_then(OtelModelValue::as_str)
            .map(str::to_owned),
        metadata: flatten_metadata(metadata),
    }
}

mod tests;
