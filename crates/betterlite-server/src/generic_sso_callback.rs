use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_claims::handler::JwtHandler;
use betterlite_auth::oidc_token_exchange::decode::ProviderClaims;
use betterlite_auth::sso_mappings::repository::SsoSettingsRows;
use betterlite_auth::sso_types::identity::OpenIdIdentity;
use betterlite_config::generic_sso_callback::GenericSsoCallbackSettings;
use betterlite_config::sso_claims::ClaimAttributes;
use betterlite_config::sso_mappings::SsoMappingSources;
use betterlite_config::sso_pkce::PkceSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;

pub mod convert;
pub mod exchange;
pub mod failure;
mod setup;

mod tests;

use crate::sso_pkce::verifier_cache::{PkceCaches, VerifierCache};

use self::failure::{GenericSsoError, pkce_hint_message, pkce_hint_required};
use self::setup::SsoSetup;

// the callback URL and the browser-bound cookie the PKCE state guard compares
pub struct GenericSsoCallback<'callback> {
    pub url: &'callback str,
    pub sso_state_cookie: Option<&'callback str>,
}

// the settings, handlers and mappings one generic login runs with
pub struct GenericSsoConfig<'config> {
    pub client_id: &'config str,
    pub redirect_url: &'config str,
    pub sources: &'config SsoMappingSources,
    pub pkce: &'config PkceSettings,
    pub callback_settings: &'config GenericSsoCallbackSettings,
    pub attributes: &'config ClaimAttributes,
    pub jwt_handler: &'config JwtHandler,
    pub sso_jwt_handler: Option<&'config JwtHandler>,
}

#[derive(Debug)]
pub struct SsoExchangeResult {
    pub identity: OpenIdIdentity,
    pub received_response: ProviderClaims,
    pub access_token_payload: Option<TypedValue>,
    pub sso_assertion: Option<SsoIdentityAssertion>,
}

// get_generic_sso_response: set up outside the wrapper, exchange inside it, classify the failure
pub async fn get_generic_sso_response<Rows, Redis, User>(
    transport: &ProxiedClient,
    callback: &GenericSsoCallback<'_>,
    config: &GenericSsoConfig<'_>,
    rows: &Rows,
    caches: &PkceCaches<'_, Redis, User>,
) -> Result<SsoExchangeResult, GenericSsoError>
where
    Rows: SsoSettingsRows,
    Redis: VerifierCache,
    User: VerifierCache,
{
    let setup = setup::load(config, rows).await?;
    match exchange::exchange(transport, callback, config, &setup, caches).await {
        Ok(result) => Ok(result),
        Err(error) => Err(handle_generic_sso_error(error, config, &setup)),
    }
}

fn handle_generic_sso_error(
    error: GenericSsoError,
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
) -> GenericSsoError {
    let message = error.to_string();
    if pkce_hint_required(&message, config.pkce.use_pkce) {
        return GenericSsoError::PkceHint {
            message: pkce_hint_message(
                &message,
                &setup.settings.authorization_endpoint,
                &setup.settings.token_endpoint,
            ),
        };
    }
    // the diagnostic names the failure only: the additional header values stay out of the log
    tracing::error!(error = %error, "SSO authentication failed");
    error
}
