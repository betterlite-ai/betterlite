use std::sync::Arc;

use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use url::Url;

use crate::credential_contracts::auth_config::id_jag::IdJagConfig;
use crate::credential_contracts::auth_config::key_source::{ApiKeyConfig, ApiKeySource};
use crate::credential_contracts::auth_config::oauth::{
    ClientCredentialsConfig, TokenExchangeConfig,
};
use crate::credential_contracts::errors::{CredError, CredResult, Unauthorized};
use crate::credential_contracts::header_carrier::HeaderCarrier;
use crate::credential_contracts::httpx_auth::NoOpAuth;
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credentials::ResolvedAuth;

use crate::outbound_credentials::auth::{ClientCredentialsAuth, StaticAuth, static_auth};
use crate::outbound_credentials::collaborators::{
    AssertionStore, ClientCredentialsSource, CredentialRequest, ExchangeRequest, OAuthToken,
    OAuthTokenStore, TokenExchanger, TokenStoreUnavailable,
};

use super::Collaborators;
use super::id_jag;

pub(super) fn no_credential(server: &ServerSpec) -> CredResult<Arc<dyn ResolvedAuth>> {
    let Ok(url) = Url::parse(&server.resource) else {
        return Ok(Arc::new(NoOpAuth));
    };
    if url.username().is_empty() && url.password().is_none() {
        return Ok(Arc::new(NoOpAuth));
    }
    Err(CredError::UrlCredentialsNotAllowed)
}

pub(super) fn api_key(config: &ApiKeyConfig) -> CredResult<Arc<dyn ResolvedAuth>> {
    match &config.key_source {
        ApiKeySource::Shared(shared) => static_auth(&config.header, shared.value.expose_secret()),
        ApiKeySource::Byok => Err(CredError::NotImplemented(
            "api_key BYOK source not implemented yet".to_owned(),
        )),
    }
}

pub(super) fn passthrough(subject: &Subject) -> CredResult<Arc<dyn ResolvedAuth>> {
    let Some(inbound) = &subject.inbound_token else {
        return Ok(Arc::new(NoOpAuth));
    };
    let auth = StaticAuth::new("Authorization", inbound.expose_secret())?;
    Ok(Arc::new(auth))
}

// Where the caller's own bearer comes from: an SSO store for an authorization-code server, an OBO exchange for a token-exchange one.
pub(super) enum CallerToken<'call> {
    Stored(&'call dyn OAuthTokenStore),
    Exchanged(&'call dyn TokenExchanger, &'call TokenExchangeConfig),
}

pub(super) async fn user_token(
    subject: &Subject,
    server: &ServerSpec,
    header: &HeaderCarrier,
    caller_token: CallerToken<'_>,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let token = match caller_token {
        CallerToken::Stored(store) => {
            let Some(token) = authz_token(subject, server, store).await else {
                return Err(unauthorized(
                    "Authorization required: complete the OAuth flow for this server.",
                    None,
                ));
            };
            token
        }
        CallerToken::Exchanged(exchanger, config) => {
            let Some(inbound) = &subject.inbound_token else {
                return Err(unauthorized(
                    "Token exchange requires a caller token to exchange (OBO).",
                    Some("Bearer error=\"invalid_request\""),
                ));
            };
            let request = ExchangeRequest {
                subject_token: inbound,
                server,
                config,
                tenant_id: &subject.tenant_id,
            };
            exchanger.exchange(request).await?
        }
    };
    static_auth(header, token.access_token.expose_secret())
}

pub(super) async fn client_credentials(
    server_id: &str,
    config: &ClientCredentialsConfig,
    source: &Arc<dyn ClientCredentialsSource>,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let request = CredentialRequest { server_id, config };
    let token = source.get(request).await?;
    let auth = ClientCredentialsAuth::new(
        Arc::clone(source),
        server_id.to_owned(),
        config.clone(),
        token.access_token,
    );
    Ok(Arc::new(auth))
}

pub(super) async fn id_jag(
    subject: &Subject,
    server: &ServerSpec,
    config: &IdJagConfig,
    collaborators: &Collaborators,
    now: DateTime<Utc>,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let store: &dyn AssertionStore = &*collaborators.sso_assertion_store;
    let subject_token = id_jag::subject_token(subject, store, now).await?;
    id_jag::exchange(subject, server, config, &subject_token, collaborators).await
}

// A store outage reads as no token, so the caller gets the OAuth challenge instead of a 503 it cannot fix by signing in.
pub(super) async fn authz_token(
    subject: &Subject,
    server: &ServerSpec,
    store: &dyn OAuthTokenStore,
) -> Option<OAuthToken> {
    let fetched: Result<Option<OAuthToken>, TokenStoreUnavailable> =
        store.fetch(&subject.subject_id, &server.server_id).await;
    fetched.unwrap_or_default()
}

fn unauthorized(detail: &str, www_authenticate: Option<&str>) -> CredError {
    let challenge = Unauthorized {
        detail: detail.to_owned(),
        www_authenticate: www_authenticate.map(str::to_owned),
        body: None,
        claims: None,
    };
    CredError::Unauthorized(challenge)
}
