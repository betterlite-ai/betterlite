use betterlite_auth::oauth_client::error::{OAuthClientError, OAuthErrorResponse};
use betterlite_auth::oidc_token_exchange::exchange::{
    OidcTokenExchangeError, OidcTokenExchangeErrorResponse,
};
use betterlite_auth::sso_claims::claims::SsoClaimsError;
use betterlite_auth::sso_mappings::headers::HeaderError;
use betterlite_auth::sso_mappings::roles::RoleMappingError;
use betterlite_auth::sso_mappings::settings::MissingVariable;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use serde::Serialize;
use thiserror::Error;

use crate::generic_sso_callback::convert::ResponseError;
use crate::generic_sso_callback::exchange::{CallbackGuardError, HeaderBuildError};
use crate::sso_pkce::{PkceError, PkceErrorResponse};

pub const GENERIC_CLIENT_USE_PKCE_PARAM: &str = "GENERIC_CLIENT_USE_PKCE";

// LiteLLM re-raises either the hint ProxyException or the original one, so a caller reads exactly these four fields
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenericSsoErrorResponse {
    pub status: u16,
    pub error_type: ProxyErrorTypes,
    pub param: Option<String>,
    pub code: String,
}

#[derive(Debug, Error)]
pub enum GenericSsoError {
    #[error(transparent)]
    MissingVariable(#[from] MissingVariable),
    #[error(transparent)]
    RoleMapping(#[from] RoleMappingError),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error(transparent)]
    Guard(#[from] CallbackGuardError),
    #[error(transparent)]
    HeaderBuild(#[from] HeaderBuildError),
    #[error(transparent)]
    Pkce(#[from] PkceError),
    #[error(transparent)]
    OAuth(#[from] OAuthClientError),
    #[error(transparent)]
    TokenExchange(#[from] OidcTokenExchangeError),
    #[error(transparent)]
    Claims(#[from] SsoClaimsError),
    #[error(transparent)]
    Response(#[from] ResponseError),
    #[error("{message}")]
    PkceHint { message: String },
}

impl GenericSsoError {
    #[must_use]
    pub fn response(&self) -> GenericSsoErrorResponse {
        match self {
            Self::MissingVariable(error) => GenericSsoErrorResponse {
                status: 500,
                error_type: ProxyErrorTypes::AuthError,
                param: error.0.param.clone(),
                code: "500".to_owned(),
            },
            Self::RoleMapping(_)
            | Self::Header(_)
            | Self::HeaderBuild(_)
            | Self::Claims(_)
            | Self::Response(_) => {
                response_of(500, ProxyErrorTypes::InternalServerError, None, "500")
            }
            Self::Guard(error) => error.response(),
            Self::Pkce(error) => from_pkce(error.response()),
            Self::OAuth(error) => from_oauth(error.response()),
            Self::TokenExchange(error) => from_token_exchange(error.response()),
            Self::PkceHint { .. } => pkce_hint_response(),
        }
    }
}

fn from_pkce(response: PkceErrorResponse) -> GenericSsoErrorResponse {
    response_of(
        response.status,
        response.error_type,
        response.param,
        response.code,
    )
}

fn from_oauth(response: OAuthErrorResponse) -> GenericSsoErrorResponse {
    response_of(
        response.status,
        response.error_type,
        response.param,
        response.code,
    )
}

fn from_token_exchange(response: OidcTokenExchangeErrorResponse) -> GenericSsoErrorResponse {
    response_of(
        response.status,
        response.error_type,
        response.param,
        response.code,
    )
}

fn response_of(
    status: u16,
    error_type: ProxyErrorTypes,
    param: Option<&'static str>,
    code: &'static str,
) -> GenericSsoErrorResponse {
    GenericSsoErrorResponse {
        status,
        error_type,
        param: param.map(str::to_owned),
        code: code.to_owned(),
    }
}

// the pinned wrapper: PKCE asked for while switched off gets the hint, every other error its response
#[must_use]
pub fn classify_generic_sso_error(
    message: &str,
    source: GenericSsoErrorResponse,
    pkce_configured: bool,
) -> GenericSsoErrorResponse {
    if pkce_hint_required(message, pkce_configured) {
        return pkce_hint_response();
    }
    source
}

// "PKCE" is matched as written and "code verifier" without case, exactly as the pinned wrapper does
#[must_use]
pub fn pkce_hint_required(message: &str, pkce_configured: bool) -> bool {
    !pkce_configured
        && (message.contains("PKCE") || message.to_lowercase().contains("code verifier"))
}

#[must_use]
pub fn pkce_hint_response() -> GenericSsoErrorResponse {
    response_of(
        401,
        ProxyErrorTypes::AuthError,
        Some(GENERIC_CLIENT_USE_PKCE_PARAM),
        "401",
    )
}

#[must_use]
pub fn pkce_hint_message(
    original: &str,
    authorization_endpoint: &str,
    token_endpoint: &str,
) -> String {
    let is_okta = authorization_endpoint.to_lowercase().contains("okta")
        || token_endpoint.to_lowercase().contains("okta");
    let provider_name = if is_okta {
        "Okta"
    } else {
        "Your OAuth provider"
    };
    let okta_steps = if is_okta {
        "For AWS ECS: Add the environment variable to your task definition.\n\
For Docker: Add -e GENERIC_CLIENT_USE_PKCE=true to your docker run command.\n\
For .env file: Add GENERIC_CLIENT_USE_PKCE=true to your .env file.\n\n"
    } else {
        ""
    };
    format!(
        "SSO authentication failed: {provider_name} requires PKCE (Proof Key for Code Exchange) \
but it's not enabled in your LiteLLM configuration.\n\n\
SOLUTION: Add this environment variable and restart your proxy:\n  \
GENERIC_CLIENT_USE_PKCE=true\n\n\
{okta_steps}Original error: {original}"
    )
}
