use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::oauth_client::OAuthClient;
use betterlite_auth::oauth_client::document::OAuthDiscoveryDocument;
use betterlite_auth::oauth_client::exchange::{
    CallbackRequest, ExchangeRequest, OAuthExchange, exchange,
};
use betterlite_auth::oidc_token_exchange::decode::ProviderClaims;
use betterlite_auth::oidc_token_exchange::exchange::{TokenExchangeRequest, exchange_pkce_code};
use betterlite_auth::sso_claims::access_token::process_sso_jwt_access_token;
use betterlite_auth::sso_claims::convert::ConversionConfig;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_http::proxy::ProxiedClient;
use betterlite_mcp::sso_assertions::assertion::assertion_from_sso_login;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use thiserror::Error;
use url::Url;

use crate::sso_pkce::token_exchange::{PkceVerifier, token_exchange_parameters};
use crate::sso_pkce::verifier_cache::{PkceCaches, VerifierCache};

use super::convert::{
    ConvertedResponse, ResponseError, ResponseTokens, convert_response, text_document,
};
use super::failure::{GenericSsoError, GenericSsoErrorResponse};
use super::setup::SsoSetup;
use super::{GenericSsoCallback, GenericSsoConfig, SsoExchangeResult};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CallbackGuardError {
    #[error("Invalid OAuth state parameter: it does not match the browser-bound state cookie")]
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

async fn pkce_exchange(
    transport: &ProxiedClient,
    callback: &GenericSsoCallback<'_>,
    config: &GenericSsoConfig<'_>,
    setup: &SsoSetup,
    verifier: PkceVerifier,
) -> Result<Exchanged, GenericSsoError> {
    let PkceVerifier {
        cache_key,
        code_verifier,
    } = verifier;
    let request = pkce_token_request(callback, config, setup, code_verifier)?;
    let claims = exchange_pkce_code(transport, &request).await?;
    let assertion_id_token = claims.get("id_token").cloned();
    let assertion_refresh_token = claims.get("refresh_token").cloned();
    let access_token = claims
        .get("access_token")
        .and_then(|value| value.json.as_str())
        .map(str::to_owned);
    Ok(Exchanged {
        claims,
        fallback: ResponseTokens {
            id_token: None,
            access_token: None,
        },
        assertion_id_token,
        assertion_refresh_token,
        access_token,
        verifier_key: Some(cache_key),
    })
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

async fn oauth_exchange(
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
    let exchanged = exchange(&client, transport, &discovery, &request).await?;
    exchanged_from_oauth(&exchanged)
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

// the non-PKCE client's tokens stand in for a response that carries none, and feed the assertion
fn exchanged_from_oauth(exchanged: &OAuthExchange) -> Result<Exchanged, GenericSsoError> {
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
    Ok(Exchanged {
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
    let (identity, access_token_payload) = process_sso_jwt_access_token(
        exchanged.access_token.as_deref(),
        config.sso_jwt_handler,
        Some(converted.identity),
        setup.role_mappings.as_ref(),
        &config.attributes.role,
    )?;
    let Some(identity) = identity else {
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

fn verify_callback_guards(
    state: Option<&str>,
    cookie: Option<&str>,
    code: Option<&str>,
    client_id: &str,
    token_endpoint: &str,
) -> Result<String, CallbackGuardError> {
    // a verifier implies a nonempty state, so absent or empty values fail the bound pair's compare
    let url_state = state.unwrap_or_default();
    let cookie_state = cookie.unwrap_or_default();
    if !bool::from(url_state.as_bytes().ct_eq(cookie_state.as_bytes())) {
        return Err(CallbackGuardError::State);
    }
    let Some(authorization_code) = code.filter(|value| !value.is_empty()) else {
        return Err(CallbackGuardError::Code);
    };
    if client_id.is_empty() {
        return Err(CallbackGuardError::ClientId);
    }
    if token_endpoint.is_empty() {
        return Err(CallbackGuardError::TokenEndpoint);
    }
    Ok(authorization_code.to_owned())
}

fn guard_response(status: u16, param: &'static str) -> GenericSsoErrorResponse {
    GenericSsoErrorResponse {
        status,
        error_type: ProxyErrorTypes::AuthError,
        param: Some(param.to_owned()),
        code: status.to_string(),
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

// Starlette's QueryParams.get keeps the last value a key carries
fn last_query_value(url: &str, name: &str) -> Option<String> {
    let Ok(parsed) = Url::parse(url) else {
        return None;
    };
    parsed
        .query_pairs()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .last()
}
