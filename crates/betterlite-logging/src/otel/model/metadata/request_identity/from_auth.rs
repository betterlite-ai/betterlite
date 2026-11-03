use super::super::common::{field, object, string, team_metadata};
use super::super::helpers::flatten_metadata;
use super::RequestIdentity;
use crate::otel::model::utils::coercion::OtelModelValue;

#[must_use]
pub fn from_user_api_key_auth(
    auth: &OtelModelValue,
    request_metadata: Option<&OtelModelValue>,
) -> RequestIdentity {
    let mut metadata = flatten_metadata(request_metadata.and_then(|value| object(Some(value))));
    for (key, field_name) in [
        ("user_api_key_user_id", "user_id"),
        ("user_api_key_org_id", "org_id"),
        ("user_api_key_alias", "key_alias"),
        ("user_api_key_end_user_id", "end_user_id"),
    ] {
        if let Some(value) = string(field(auth, field_name)) {
            metadata.insert(key.to_owned(), value);
        }
    }
    RequestIdentity {
        call_id: None,
        team_id: string(field(auth, "team_id")),
        team_alias: string(field(auth, "team_alias")),
        team_metadata: team_metadata(field(auth, "team_metadata")),
        key_hash: string(field(auth, "api_key")),
        end_user: string(field(auth, "end_user_id")),
        provider_model: None,
        request_route: None,
        metadata,
    }
}

mod tests;
