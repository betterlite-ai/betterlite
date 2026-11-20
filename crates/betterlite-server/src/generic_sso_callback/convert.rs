use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::oidc_token_exchange::decode::ProviderClaims;
use betterlite_auth::sso_claims::convert::{ConversionConfig, convert_generic_response};
use betterlite_auth::sso_claims::decode::merge_sso_token_claims;
use betterlite_auth::sso_types::identity::{OAUTH_TOKEN_FIELDS, OpenIdIdentity};
use betterlite_config::sso_claims::ClaimAttributes;
use thiserror::Error;

use super::failure::GenericSsoError;

#[derive(Debug, Error)]
pub enum ResponseError {
    #[error("the SSO provider response cannot be carried into the identity converter")]
    Encode(#[source] serde_json::Error),
    #[error("the SSO provider response cannot be read as a set of claims")]
    Decode(#[source] serde_json::Error),
    #[error("the converted SSO identity did not survive access-token processing")]
    IdentityMissing,
}

// the tokens the per-login provider client holds, which the convertor falls back to
pub(super) struct ResponseTokens {
    pub(super) id_token: Option<String>,
    pub(super) access_token: Option<String>,
}

pub(super) struct ConvertedResponse {
    pub(super) identity: OpenIdIdentity,
    pub(super) received_response: ProviderClaims,
}

// response_convertor: the response token wins over the client's; token claims merge only when asked
pub(super) fn convert_response(
    claims: &ProviderClaims,
    tokens: &ResponseTokens,
    include_token_claims: bool,
    config: &ConversionConfig<'_>,
    attributes: &ClaimAttributes,
) -> Result<ConvertedResponse, GenericSsoError> {
    let id_token = text_claim(claims.get("id_token")).or(tokens.id_token.as_deref());
    let access_token = text_claim(claims.get("access_token")).or(tokens.access_token.as_deref());
    let claims = if include_token_claims {
        merge_sso_token_claims(claims, id_token, access_token)
    } else {
        claims.clone()
    };
    let received_response = without_token_fields(&claims);
    let identity = convert_generic_response(&claims_document(&claims)?, config, attributes)?;
    Ok(ConvertedResponse {
        identity,
        received_response,
    })
}

pub(super) fn text_document(text: &str) -> Result<TypedValue, ResponseError> {
    Ok(TypedValue {
        json: serde_json::to_value(text).map_err(ResponseError::Encode)?,
    })
}

fn claims_document(claims: &ProviderClaims) -> Result<TypedValue, ResponseError> {
    Ok(TypedValue {
        json: serde_json::to_value(claims).map_err(ResponseError::Encode)?,
    })
}

fn text_claim(value: Option<&TypedValue>) -> Option<&str> {
    value
        .and_then(|found| found.json.as_str())
        .filter(|text| !text.is_empty())
}

fn without_token_fields(claims: &ProviderClaims) -> ProviderClaims {
    claims
        .iter()
        .filter(|(name, _)| !OAUTH_TOKEN_FIELDS.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
