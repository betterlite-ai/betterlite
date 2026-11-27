mod tests;

use betterlite_config::sections::microsoft_sso::{CLIENT_SECRET_ENV, Settings};
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use http::HeaderMap;
use thiserror::Error;

use crate::oauth_client::OAuthClient;
use crate::oauth_client::document::OAuthDiscoveryDocument;
use crate::oauth_client::error::OAuthErrorResponse;

pub const MICROSOFT_SCOPES: [&str; 3] = ["openid", "User.Read", "email"];
pub const DEFAULT_TENANT: &str = "common";

pub struct MicrosoftSsoClient {
    pub oauth: OAuthClient,
    pub tenant: String,
    pub discovery: OAuthDiscoveryDocument,
}

#[derive(Debug, Error)]
pub enum MicrosoftClientError {
    #[error("MICROSOFT_CLIENT_SECRET not set. Set it in .env file")]
    MissingClientSecret,
}

impl MicrosoftClientError {
    #[must_use]
    pub const fn response(&self) -> OAuthErrorResponse {
        match self {
            Self::MissingClientSecret => OAuthErrorResponse {
                status: 500,
                error_type: ProxyErrorTypes::AuthError,
                param: Some(CLIENT_SECRET_ENV),
                code: "500",
            },
        }
    }
}

#[must_use]
pub fn microsoft_default_discovery(tenant: &str) -> OAuthDiscoveryDocument {
    OAuthDiscoveryDocument {
        authorization_endpoint: format!(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
        ),
        token_endpoint: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
        userinfo_endpoint: "https://graph.microsoft.com/v1.0/me".to_owned(),
    }
}

pub fn microsoft_sso_client(
    settings: &Settings,
    client_id: &str,
    redirect_url: &str,
) -> Result<MicrosoftSsoClient, MicrosoftClientError> {
    let Some(client_secret) = settings.client_secret.as_ref() else {
        return Err(MicrosoftClientError::MissingClientSecret);
    };
    let tenant = settings
        .tenant
        .as_deref()
        .filter(|tenant| !tenant.is_empty())
        .map_or_else(|| DEFAULT_TENANT.to_owned(), str::to_owned);
    let discovery = discovery_with_overrides(settings, &tenant);
    let oauth = OAuthClient {
        client_id: client_id.to_owned(),
        client_secret: client_secret.clone(),
        redirect_uri: Some(redirect_url.to_owned()),
        scopes: MICROSOFT_SCOPES
            .iter()
            .copied()
            .map(str::to_owned)
            .collect(),
        allow_insecure_http: true,
        additional_headers: HeaderMap::new(),
    };
    Ok(MicrosoftSsoClient {
        oauth,
        tenant,
        discovery,
    })
}

fn discovery_with_overrides(settings: &Settings, tenant: &str) -> OAuthDiscoveryDocument {
    let mut document = microsoft_default_discovery(tenant);
    let mut custom = false;
    for (override_value, endpoint) in [
        (
            &settings.authorization_endpoint,
            &mut document.authorization_endpoint,
        ),
        (&settings.token_endpoint, &mut document.token_endpoint),
        (&settings.userinfo_endpoint, &mut document.userinfo_endpoint),
    ] {
        let Some(value) = override_value.as_deref().filter(|value| !value.is_empty()) else {
            continue;
        };
        *endpoint = value.to_owned();
        custom = true;
    }
    if custom {
        tracing::debug!(
            "Using custom Microsoft SSO endpoints - authorization: {}, token: {}, userinfo: {}",
            document.authorization_endpoint,
            document.token_endpoint,
            document.userinfo_endpoint,
        );
    }
    document
}
