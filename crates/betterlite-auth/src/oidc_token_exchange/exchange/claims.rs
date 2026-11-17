use crate::jwt_claims::TypedValue;
use crate::jwt_claims::decode::unverified_claims;
use crate::oidc_token_exchange::decode::{ProviderClaims, TokenResponse};
use crate::sso_types::identity::OAUTH_TOKEN_FIELDS;

use super::{ID_TOKEN_FIELD, OidcTokenExchangeError};

// LiteLLM decodes the id_token payload without checking its signature, since it came from the configured exchange
pub(super) fn fallback_claims(
    token_response: &TokenResponse,
) -> Result<ProviderClaims, OidcTokenExchangeError> {
    let id_token = token_response
        .claims
        .get(ID_TOKEN_FIELD)
        .and_then(|value| value.json.as_str())
        .filter(|token| !token.is_empty());
    let Some(id_token) = id_token else {
        return Err(OidcTokenExchangeError::UserinfoUnavailable);
    };
    let Some(document) = unverified_claims(id_token) else {
        tracing::error!("the SSO id_token is not a decodable JWT");
        return Err(OidcTokenExchangeError::IdTokenInvalid);
    };
    if document
        .json
        .as_object()
        .is_none_or(|fields| fields.is_empty())
    {
        tracing::warn!("the SSO id_token payload held no identity claims");
        return Err(OidcTokenExchangeError::UserinfoUnavailable);
    }
    serde_json::from_value(document.json).map_err(OidcTokenExchangeError::IdTokenClaims)
}

// LiteLLM's `merged = {**token_response, **userinfo}` then its loop over the bearer fields: a token value wins, an explicit null removes the key, an absent one keeps the userinfo value
pub(super) fn merge_claims(
    token_response: ProviderClaims,
    userinfo: ProviderClaims,
) -> ProviderClaims {
    let bearer: Vec<(&str, Option<TypedValue>)> = OAUTH_TOKEN_FIELDS
        .into_iter()
        .map(|field| (field, token_response.get(field).cloned()))
        .collect();
    let mut merged = token_response;
    for (name, value) in userinfo {
        merged.insert(name, value);
    }
    for (field, value) in bearer {
        if let Some(value) = value {
            if value.json.is_null() {
                merged.remove(field);
            } else {
                merged.insert(field.to_owned(), value);
            }
        }
    }
    merged
}
