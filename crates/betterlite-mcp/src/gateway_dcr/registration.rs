mod tests;

use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_http::user_url::error::UrlParseError;
use betterlite_http::user_url::split::urlparse;
use chrono::{DateTime, Utc};
use serde::Serialize;
use thiserror::Error;

use crate::gateway_dcr::contracts::{
    GATEWAY_DCR_CLIENT_ID_PREFIX, MAX_CLIENT_ID_LENGTH, MAX_REDIRECT_URI_LENGTH, MAX_REDIRECT_URIS,
    supported_grant_types,
};
use crate::gateway_dcr::models::GatewayDcrClient;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::sealed::{SealError, seal};
use crate::redirect_trust::shape::{
    RedirectShape, is_loopback_redirect_host, validate_redirect_uri_shape,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_id_issued_at: i64,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: &'static str,
    pub grant_types: Vec<&'static str>,
    pub response_types: [&'static str; 1],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Registration {
    Registered(RegisteredClient),
    Refused(OauthError),
}

impl Registration {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Registered(_) => 201,
            Self::Refused(error) => error.status,
        }
    }
}

#[derive(Debug, Error)]
pub enum RegistrationRefusal {
    #[error("redirect_uris must be a list of 1 to 4 URIs")]
    RedirectUrisList,
    #[error("each redirect URI must be a string of at most 256 characters")]
    RedirectUriItem,
    #[error("each redirect URI must be a valid URL")]
    RedirectUriUnparsable(#[source] UrlParseError),
    #[error("{description}")]
    RedirectUriShape { description: String },
    #[error(
        "each redirect URI must be https, http on a loopback host, or a registered native callback"
    )]
    RedirectUriScheme,
    #[error("registered metadata is too large")]
    MetadataTooLarge,
    #[error("the client registration could not be sealed")]
    SealFailed(#[source] SealError),
}

#[must_use]
pub fn register_aggregate_client(
    body: &BTreeMap<String, JsonValue>,
    token_exchange_available: bool,
    now: DateTime<Utc>,
    cipher: &DbCipher,
    native_callbacks: &[String],
) -> Registration {
    let redirect_uris = match redirect_uris_from(body) {
        Ok(redirect_uris) => redirect_uris,
        Err(refusal) => return refused(&refusal),
    };
    for uri in &redirect_uris {
        if let Err(refusal) = check_redirect_uri(uri, native_callbacks) {
            return refused(&refusal);
        }
    }
    let issued_at = now.timestamp();
    let client = GatewayDcrClient {
        redirect_uris: redirect_uris.clone(),
        iat: issued_at,
    };
    let client_id = match seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client, cipher) {
        Ok(client_id) => client_id,
        Err(error) => return refused(&RegistrationRefusal::SealFailed(error)),
    };
    if client_id.chars().count() > MAX_CLIENT_ID_LENGTH {
        return refused(&RegistrationRefusal::MetadataTooLarge);
    }
    Registration::Registered(RegisteredClient {
        client_id,
        client_id_issued_at: issued_at,
        redirect_uris,
        token_endpoint_auth_method: "none",
        grant_types: supported_grant_types(token_exchange_available),
        response_types: ["code"],
    })
}

fn redirect_uris_from(
    body: &BTreeMap<String, JsonValue>,
) -> Result<Vec<String>, RegistrationRefusal> {
    let Some(JsonValue::Array(items)) = body.get("redirect_uris") else {
        return Err(RegistrationRefusal::RedirectUrisList);
    };
    if items.is_empty() || items.len() > MAX_REDIRECT_URIS {
        return Err(RegistrationRefusal::RedirectUrisList);
    }
    let mut redirect_uris = Vec::with_capacity(items.len());
    for item in items {
        let JsonValue::String(uri) = item else {
            return Err(RegistrationRefusal::RedirectUriItem);
        };
        if uri.chars().count() > MAX_REDIRECT_URI_LENGTH {
            return Err(RegistrationRefusal::RedirectUriItem);
        }
        redirect_uris.push(uri.clone());
    }
    Ok(redirect_uris)
}

fn check_redirect_uri(uri: &str, native_callbacks: &[String]) -> Result<(), RegistrationRefusal> {
    let parsed = urlparse(uri, "").map_err(RegistrationRefusal::RedirectUriUnparsable)?;
    let shape = validate_redirect_uri_shape(&parsed, native_callbacks).map_err(|rejected| {
        RegistrationRefusal::RedirectUriShape {
            description: rejected.description,
        }
    })?;
    let is_web =
        parsed.scheme == "https" || (parsed.scheme == "http" && is_loopback_redirect_host(&parsed));
    if shape == RedirectShape::NativeCallback || is_web {
        return Ok(());
    }
    Err(RegistrationRefusal::RedirectUriScheme)
}

fn refused(refusal: &RegistrationRefusal) -> Registration {
    let (status, error) = match refusal {
        RegistrationRefusal::MetadataTooLarge => (400, OauthErrorCode::InvalidClientMetadata),
        RegistrationRefusal::SealFailed(_) => (500, OauthErrorCode::ServerError),
        RegistrationRefusal::RedirectUrisList
        | RegistrationRefusal::RedirectUriItem
        | RegistrationRefusal::RedirectUriUnparsable(_)
        | RegistrationRefusal::RedirectUriShape { .. }
        | RegistrationRefusal::RedirectUriScheme => (400, OauthErrorCode::InvalidRedirectUri),
    };
    Registration::Refused(OauthError {
        status,
        error,
        description: refusal.to_string(),
    })
}
