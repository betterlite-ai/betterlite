use std::collections::BTreeMap;

use secrecy::SecretString;
use thiserror::Error;

use crate::jwt_claims::TypedValue;

// LiteLLM reads the token response and the userinfo response with the same JSON call, so one bound covers both.
pub const MAX_OIDC_RESPONSE_BYTES: usize = 0x0010_0000;

const ACCESS_TOKEN_FIELD: &str = "access_token";

pub type ProviderClaims = BTreeMap<String, TypedValue>;

// the tag in front of each payload in the fuzz corpus, so one target replays both decoders
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcResponseKind {
    Token,
    Userinfo,
}

#[derive(Debug, Error)]
pub enum OidcResponseError {
    #[error("the provider response is {bytes} bytes, over the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the provider response is not JSON")]
    Json(#[source] serde_json::Error),
    #[error("the provider response is not a JSON object")]
    NotAnObject,
    #[error("the provider response carries no non-empty access_token string")]
    MissingAccessToken,
    #[error("the provider response claims cannot be read as a JSON object")]
    Claims(#[source] serde_json::Error),
    #[error("the provider response tag {tag} names no response kind")]
    UnknownKind { tag: u8 },
}

#[derive(Debug)]
pub enum OidcResponse {
    Token(TokenResponse),
    Userinfo(UserinfoDocument),
}

#[derive(Debug)]
pub struct TokenResponse {
    pub claims: ProviderClaims,
    pub access_token: SecretString,
}

// LiteLLM keeps a falsy userinfo body as no claims, rejects a truthy non-object body, and merges any other object
#[derive(Debug)]
pub enum UserinfoDocument {
    Claims(ProviderClaims),
    Empty,
    NotAnObject,
}

impl TryFrom<u8> for OidcResponseKind {
    type Error = OidcResponseError;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::Token),
            1 => Ok(Self::Userinfo),
            unknown => Err(OidcResponseError::UnknownKind { tag: unknown }),
        }
    }
}

impl OidcResponseKind {
    pub fn decode(self, payload: &[u8]) -> Result<OidcResponse, OidcResponseError> {
        match self {
            Self::Token => decode_oidc_token_response(payload).map(OidcResponse::Token),
            Self::Userinfo => decode_oidc_userinfo(payload).map(OidcResponse::Userinfo),
        }
    }
}

// LiteLLM's _validate_token_response: the body must be a JSON object whose access_token is a non-empty string
pub fn decode_oidc_token_response(bytes: &[u8]) -> Result<TokenResponse, OidcResponseError> {
    let document = decode_document(bytes)?;
    if !document.json.is_object() {
        return Err(OidcResponseError::NotAnObject);
    }
    let Some(issued) = document
        .json
        .get(ACCESS_TOKEN_FIELD)
        .and_then(|value| value.as_str())
        .filter(|token| !token.is_empty())
    else {
        return Err(OidcResponseError::MissingAccessToken);
    };
    let access_token = SecretString::from(issued.to_owned());
    let claims = serde_json::from_value(document.json).map_err(OidcResponseError::Claims)?;
    Ok(TokenResponse {
        claims,
        access_token,
    })
}

// LiteLLM's _get_pkce_userinfo keeps a falsy body apart from a body it cannot merge
pub fn decode_oidc_userinfo(bytes: &[u8]) -> Result<UserinfoDocument, OidcResponseError> {
    let document = decode_document(bytes)?;
    if is_empty_document(&document) {
        return Ok(UserinfoDocument::Empty);
    }
    if !document.json.is_object() {
        return Ok(UserinfoDocument::NotAnObject);
    }
    serde_json::from_value(document.json)
        .map(UserinfoDocument::Claims)
        .map_err(OidcResponseError::Claims)
}

fn decode_document(bytes: &[u8]) -> Result<TypedValue, OidcResponseError> {
    if bytes.len() > MAX_OIDC_RESPONSE_BYTES {
        return Err(OidcResponseError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_OIDC_RESPONSE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(OidcResponseError::Json)
}

// Python's falsy test, which decides whether LiteLLM falls back to the id_token
pub(super) fn is_empty_document(document: &TypedValue) -> bool {
    let json = &document.json;
    if json.is_null()
        || json.as_bool() == Some(false)
        || json.as_f64() == Some(0.0)
        || json.as_str() == Some("")
    {
        return true;
    }
    if let Some(items) = json.as_array() {
        return items.is_empty();
    }
    if let Some(fields) = json.as_object() {
        return fields.is_empty();
    }
    false
}
