use betterlite_auth::oidc_token_exchange::exchange::{TokenExchangeRequest, exchange_pkce_code};
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;

use crate::generic_sso_callback::failure::GenericSsoError;
use crate::generic_sso_callback::setup::SsoSetup;
use crate::generic_sso_callback::{GenericSsoCallback, GenericSsoConfig};
use crate::sso_pkce::token_exchange::PkceVerifier;

use super::Exchanged;
use super::guards::{last_query_value, verify_callback_guards};

pub(super) async fn pkce_exchange(
    transport: &ProxiedClient,
    callback: &GenericSsoCallback<'_>,
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
    verifier: PkceVerifier,
) -> Result<Exchanged, GenericSsoError> {
    let request = pkce_token_request(callback, config, setup, verifier.code_verifier)?;
    let claims = exchange_pkce_code(transport, &request).await?;
    Ok(Exchanged::from_pkce(claims, verifier.cache_key))
}

// the guards run before the request exists, so a refused callback costs no provider round trip
fn pkce_token_request(
    callback: &GenericSsoCallback<'_>,
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
    code_verifier: SecretString,
) -> Result<TokenExchangeRequest, GenericSsoError> {
    let state = last_query_value(callback.url, "state");
    let code = last_query_value(callback.url, "code");
    let authorization_code = verify_callback_guards(
        state.as_deref(),
        callback.sso_state_cookie,
        code.as_deref(),
        config.client_id,
        &setup.settings.token_endpoint,
    )?;
    Ok(TokenExchangeRequest {
        authorization_code,
        code_verifier,
        client_id: config.client_id.to_owned(),
        client_secret: Some(setup.settings.client_secret.clone()),
        token_endpoint: setup.settings.token_endpoint.clone(),
        userinfo_endpoint: Some(setup.settings.userinfo_endpoint.clone()),
        include_client_id: setup.settings.include_client_id,
        redirect_url: Some(config.redirect_url.to_owned()),
        additional_headers: setup
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    })
}
