use betterlite_auth::oauth_client::OAuthClient;
use betterlite_auth::oauth_client::discovery::{DiscoverySource, fetch_discovery};
use betterlite_auth::oauth_client::document::OAuthUserInfo;
use betterlite_auth::oauth_client::error::{OAuthClientError, OAuthErrorResponse};
use betterlite_auth::oauth_client::exchange::{CallbackRequest, ExchangeRequest, exchange};
use betterlite_auth::oauth_client::google::{
    GOOGLE_DISCOVERY_URL, GOOGLE_SCOPES, GoogleIdentityError, google_open_id,
};
use betterlite_auth::sso_types::identity::OpenIdIdentity;
use betterlite_config::google_sso::GOOGLE_CLIENT_SECRET_ENV;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_http::proxy::ProxiedClient;
use http::HeaderMap;
use secrecy::SecretString;
use thiserror::Error;

mod tests;

pub struct GoogleSsoConfig {
    pub client_id: String,
    pub redirect_url: String,
    pub client_secret: Option<SecretString>,
    // None reaches Google's discovery document; a test points it at a local provider.
    pub discovery_url: Option<String>,
    pub allow_insecure_http: bool,
}

impl GoogleSsoConfig {
    #[must_use]
    pub fn discovery_url(&self) -> &str {
        self.discovery_url
            .as_deref()
            .unwrap_or(GOOGLE_DISCOVERY_URL)
    }
}

pub struct GoogleCallback<'a> {
    pub url: &'a str,
    pub sso_state_cookie: Option<&'a str>,
}

#[derive(Debug)]
pub enum GoogleSsoResponse {
    Converted(Box<OpenIdIdentity>),
    Raw(OAuthUserInfo),
}

#[derive(Debug, Error)]
pub enum GoogleSsoError {
    #[error("GOOGLE_CLIENT_SECRET not set. Set it in .env file")]
    MissingClientSecret,
    #[error(transparent)]
    Client(#[from] OAuthClientError),
    #[error(transparent)]
    Identity(#[from] GoogleIdentityError),
}

impl GoogleSsoError {
    #[must_use]
    pub const fn response(&self) -> OAuthErrorResponse {
        match self {
            Self::MissingClientSecret => OAuthErrorResponse {
                status: 500,
                error_type: ProxyErrorTypes::AuthError,
                param: Some(GOOGLE_CLIENT_SECRET_ENV),
                code: "500",
            },
            Self::Client(error) => error.response(),
            Self::Identity(error) => error.response(),
        }
    }
}

pub async fn get_google_callback_response(
    transport: &ProxiedClient,
    config: &GoogleSsoConfig,
    callback: &GoogleCallback<'_>,
    return_raw_sso_response: bool,
) -> Result<GoogleSsoResponse, GoogleSsoError> {
    let Some(client_secret) = config.client_secret.as_ref() else {
        return Err(GoogleSsoError::MissingClientSecret);
    };
    let client = OAuthClient {
        client_id: config.client_id.clone(),
        client_secret: client_secret.clone(),
        redirect_uri: Some(config.redirect_url.clone()),
        scopes: GOOGLE_SCOPES.iter().copied().map(str::to_owned).collect(),
        allow_insecure_http: config.allow_insecure_http,
        additional_headers: HeaderMap::new(),
    };
    let source = DiscoverySource::WellKnown(config.discovery_url().to_owned());
    let discovery = fetch_discovery(&client, transport, &source).await?;
    let request = ExchangeRequest {
        callback: CallbackRequest {
            url: callback.url,
            sso_state_cookie: callback.sso_state_cookie,
        },
        redirect_uri: Some(config.redirect_url.as_str()),
        // fastapi-sso leaves oauthlib's include_client_id default on, so the body names the client.
        include_client_id: true,
    };
    let exchanged = exchange(&client, transport, &discovery, &request).await?;
    if return_raw_sso_response {
        return Ok(GoogleSsoResponse::Raw(exchanged.userinfo));
    }
    let identity = google_open_id(&exchanged.userinfo)?;
    Ok(GoogleSsoResponse::Converted(Box::new(identity)))
}
