use betterlite_auth::oauth_client::OAuthClient;
use betterlite_auth::oauth_client::document::OAuthDiscoveryDocument;
use betterlite_auth::oauth_client::exchange::{
    CallbackRequest, ExchangeRequest, exchange as oauth_client_exchange,
};
use betterlite_http::proxy::ProxiedClient;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::generic_sso_callback::failure::GenericSsoError;
use crate::generic_sso_callback::setup::SsoSetup;
use crate::generic_sso_callback::{GenericSsoCallback, GenericSsoConfig};

use super::Exchanged;
use super::HeaderBuildError;

pub(super) async fn oauth_exchange(
    transport: &ProxiedClient,
    callback: &GenericSsoCallback<'_>,
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
    include_client_id: bool,
) -> Result<Exchanged, GenericSsoError> {
    let client = oauth_client(config, setup)?;
    let discovery = oauth_discovery(setup);
    let request = ExchangeRequest {
        callback: CallbackRequest {
            url: callback.url,
            sso_state_cookie: callback.sso_state_cookie,
        },
        redirect_uri: Some(config.redirect_url),
        include_client_id,
    };
    let exchanged = oauth_client_exchange(&client, transport, &discovery, &request).await?;
    Exchanged::from_oauth(&exchanged)
}

fn oauth_client(
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
) -> Result<OAuthClient, GenericSsoError> {
    Ok(OAuthClient {
        client_id: config.client_id.to_owned(),
        client_secret: setup.settings.client_secret.clone(),
        redirect_uri: Some(config.redirect_url.to_owned()),
        scopes: setup.settings.scope.clone(),
        // fastapi-sso is constructed with allow_insecure_http=True for the generic provider
        allow_insecure_http: true,
        additional_headers: additional_headers(&setup.headers)?,
    })
}

fn oauth_discovery(setup: &SsoSetup) -> OAuthDiscoveryDocument {
    OAuthDiscoveryDocument {
        authorization_endpoint: setup.settings.authorization_endpoint.clone(),
        token_endpoint: setup.settings.token_endpoint.clone(),
        userinfo_endpoint: setup.settings.userinfo_endpoint.clone(),
    }
}

fn additional_headers(headers: &IndexMap<String, String>) -> Result<HeaderMap, HeaderBuildError> {
    let mut built = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::try_from(name.as_str()).map_err(|source| HeaderBuildError::Name {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::try_from(value.as_str()).map_err(|source| HeaderBuildError::Value {
                name: name.clone(),
                source,
            })?;
        built.insert(header_name, header_value);
    }
    Ok(built)
}
