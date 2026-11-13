use super::super::helpers::flatten_metadata;
use super::super::wire::MetadataMap;
use super::super::wire::payload::Auth;
use super::RequestIdentity;

#[must_use]
pub fn from_user_api_key_auth(
    auth: &Auth,
    request_metadata: Option<&MetadataMap>,
) -> RequestIdentity {
    let mut metadata = flatten_metadata(request_metadata);
    for (key, value) in [
        ("user_api_key_user_id", &auth.user_id),
        ("user_api_key_org_id", &auth.org_id),
        ("user_api_key_alias", &auth.key_alias),
        ("user_api_key_end_user_id", &auth.end_user_id),
    ] {
        if let Some(text) = value {
            metadata.insert(key.to_owned(), text.clone());
        }
    }
    RequestIdentity {
        call_id: None,
        team_id: auth.team_id.clone(),
        team_alias: auth.team_alias.clone(),
        team_metadata: auth
            .team_metadata
            .clone()
            .filter(|fields| !fields.is_empty()),
        key_hash: auth.key_hash.clone(),
        end_user: auth.end_user_id.clone(),
        provider_model: None,
        request_route: None,
        metadata,
    }
}

mod tests;
