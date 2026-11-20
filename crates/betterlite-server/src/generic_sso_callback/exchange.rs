use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::oauth_client::exchange::OAuthExchange;
use betterlite_auth::oidc_token_exchange::decode::ProviderClaims;
use betterlite_auth::sso_claims::access_token::process_sso_jwt_access_token;
use betterlite_auth::sso_claims::convert::ConversionConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_mcp::sso_assertions::assertion::assertion_from_sso_login;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use secrecy::ExposeSecret as _;
use thiserror::Error;

use crate::sso_pkce::token_exchange::token_exchange_parameters;
use crate::sso_pkce::verifier_cache::{PkceCaches, VerifierCache};

use super::convert::{
    ConvertedResponse, ResponseError, ResponseTokens, convert_response, text_document,
};
use super::failure::{GenericSsoError, GenericSsoErrorResponse};
use super::setup::SsoSetup;
use super::{GenericSsoCallback, GenericSsoConfig, SsoExchangeResult};

mod guards;
mod oauth;
mod request;

use self::guards::{guard_response, last_query_value};
use self::oauth::oauth_exchange;
use self::request::pkce_exchange;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CallbackGuardError {
    #[error("Invalid OAuth state parameter — does not match the browser-bound state cookie.")]
    State,
    #[error("Missing authorization code in callback")]
    Code,
    #[error("GENERIC_CLIENT_ID must be set when PKCE is enabled")]
    ClientId,
    #[error("GENERIC_TOKEN_ENDPOINT must be set when PKCE is enabled")]
    TokenEndpoint,
}

impl CallbackGuardError {
    #[must_use]
    pub fn response(&self) -> GenericSsoErrorResponse {
        match self {
            Self::State => guard_response(400, "state"),
            Self::Code => guard_response(400, "code"),
            Self::ClientId => guard_response(401, "GENERIC_CLIENT_ID"),
            Self::TokenEndpoint => guard_response(401, "GENERIC_TOKEN_ENDPOINT"),
        }
    }
}

#[derive(Debug, Error)]
pub enum HeaderBuildError {
    #[error("the SSO header name {name:?} is not a valid header name")]
    Name {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the SSO header {name:?} carries a byte a header value cannot hold")]
    Value {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
}

struct Exchanged {
    claims: ProviderClaims,
    fallback: ResponseTokens,
    assertion_id_token: Option<TypedValue>,
    assertion_refresh_token: Option<TypedValue>,
    access_token: Option<String>,
    verifier_key: Option<String>,
}

impl Exchanged {
    fn from_pkce(claims: ProviderClaims, cache_key: String) -> Self {
        let access_token = claims
            .get("access_token")
            .and_then(|value| value.json.as_str())
            .map(str::to_owned);
        let assertion_id_token = claims.get("id_token").cloned();
        let assertion_refresh_token = claims.get("refresh_token").cloned();
        Self {
            claims,
            fallback: ResponseTokens {
                id_token: None,
                access_token: None,
            },
            assertion_id_token,
            assertion_refresh_token,
            access_token,
            verifier_key: Some(cache_key),
        }
    }

    // the non-PKCE client's tokens stand in for a response that carries none, and feed the assertion
    fn from_oauth(exchanged: &OAuthExchange) -> Result<Self, GenericSsoError> {
        let claims: ProviderClaims = serde_json::from_value(
            serde_json::to_value(&exchanged.userinfo.fields).map_err(ResponseError::Encode)?,
        )
        .map_err(ResponseError::Decode)?;
        let id_token = exchanged
            .tokens
            .id_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned());
        let refresh_token = exchanged
            .tokens
            .refresh_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned());
        let access_token = exchanged.tokens.access_token.expose_secret().to_owned();
        Ok(Self {
            claims,
            fallback: ResponseTokens {
                id_token: id_token.clone(),
                access_token: Some(access_token.clone()),
            },
            assertion_id_token: id_token.as_deref().map(text_document).transpose()?,
            assertion_refresh_token: refresh_token.as_deref().map(text_document).transpose()?,
            access_token: Some(access_token),
            verifier_key: None,
        })
    }
}

pub(super) async fn exchange<Redis, User>(
    transport: &ProxiedClient,
    callback: &GenericSsoCallback<'_>,
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
    caches: &PkceCaches<'_, Redis, User>,
) -> Result<SsoExchangeResult, GenericSsoError>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    let state = last_query_value(callback.url, "state");
    let parameters = token_exchange_parameters(
        state.as_deref(),
        setup.settings.include_client_id,
        config.pkce,
        caches,
    )
    .await?;
    let exchanged = match parameters.verifier {
        Some(verifier) => pkce_exchange(transport, callback, config, setup, verifier).await?,
        None => {
            oauth_exchange(
                transport,
                callback,
                config,
                setup,
                parameters.include_client_id,
            )
            .await?
        }
    };
    finish(config, setup, exchanged, caches).await
}

async fn finish<Redis, User>(
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
    exchanged: Exchanged,
    caches: &PkceCaches<'_, Redis, User>,
) -> Result<SsoExchangeResult, GenericSsoError>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    let converted = convert_exchange(config, setup, &exchanged)?;
    let sso_assertion = assertion_from_sso_login(
        exchanged.assertion_id_token.as_ref(),
        exchanged.assertion_refresh_token.as_ref(),
    );
    let (processed_identity, access_token_payload) = process_sso_jwt_access_token(
        exchanged.access_token.as_deref(),
        config.sso_jwt_handler,
        Some(converted.identity),
        setup.role_mappings.as_ref(),
        &config.attributes.role,
    )?;
    let Some(identity) = processed_identity else {
        return Err(GenericSsoError::Response(ResponseError::IdentityMissing));
    };
    // the single-use verifier survives every failure above so the login can be retried
    if let Some(cache_key) = exchanged.verifier_key {
        caches.delete_verifier(&cache_key).await;
    }
    Ok(SsoExchangeResult {
        identity,
        received_response: converted.received_response,
        access_token_payload,
        sso_assertion,
    })
}

fn convert_exchange(
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
    exchanged: &Exchanged,
) -> Result<ConvertedResponse, GenericSsoError> {
    let conversion = ConversionConfig {
        jwt_handler: config.jwt_handler,
        sso_jwt_handler: config.sso_jwt_handler,
        role_mappings: setup.role_mappings.as_ref(),
        team_mappings: setup.team_mappings.as_ref(),
    };
    convert_response(
        &exchanged.claims,
        &exchanged.fallback,
        config.callback_settings.include_token_claims,
        &conversion,
        config.attributes,
    )
}
