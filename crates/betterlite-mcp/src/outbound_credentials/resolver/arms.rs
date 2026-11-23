use std::sync::Arc;

use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use url::Url;

use crate::credential_contracts::auth_config::id_jag::IdJagConfig;
use crate::credential_contracts::auth_config::key_source::{ApiKeyConfig, ApiKeySource};
use crate::credential_contracts::auth_config::oauth::{
    AuthorizationCodeConfig, ClientCredentialsConfig, TokenExchangeConfig,
};
use crate::credential_contracts::auth_spec::AuthSpecKind;
use crate::credential_contracts::errors::{CredError, CredResult, Unauthorized};
use crate::credential_contracts::httpx_auth::NoOpAuth;
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credentials::ResolvedAuth;

use crate::outbound_credentials::auth::{ClientCredentialsAuth, StaticAuth};
use crate::outbound_credentials::collaborators::{
    AssertionStore, ClientCredentialsSource, OAuthToken, OAuthTokenStore, TokenExchanger,
    TokenStoreUnavailable,
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
        ApiKeySource::Shared(shared) => {
            let (header_name, header_value) = config.header.header(shared.value.expose_secret());
            let auth = StaticAuth::new(&header_name, &header_value)?;
            Ok(Arc::new(auth))
        }
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

pub(super) async fn authorization_code(
    subject: &Subject,
    server: &ServerSpec,
    config: &AuthorizationCodeConfig,
    store: &dyn OAuthTokenStore,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let Some(token) = authz_token(subject, server, store).await else {
        return Err(unauthorized(
            "Authorization required: complete the OAuth flow for this server.",
            None,
        ));
    };
    let (header_name, header_value) = config.header.header(token.access_token.expose_secret());
    let auth = StaticAuth::new(&header_name, &header_value)?;
    Ok(Arc::new(auth))
}

pub(super) async fn has_user_token(
    subject: &Subject,
    server: &ServerSpec,
    store: &dyn OAuthTokenStore,
) -> bool {
    authz_token(subject, server, store).await.is_some()
}

pub(super) async fn client_credentials(
    server_id: &str,
    config: &ClientCredentialsConfig,
    source: &Arc<dyn ClientCredentialsSource>,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let token = source.get(server_id, config).await?;
    let auth = ClientCredentialsAuth::new(
        Arc::clone(source),
        server_id.to_owned(),
        config.clone(),
        token.access_token,
    );
    Ok(Arc::new(auth))
}

pub(super) async fn token_exchange(
    subject: &Subject,
    server: &ServerSpec,
    config: &TokenExchangeConfig,
    exchanger: &dyn TokenExchanger,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let Some(inbound) = &subject.inbound_token else {
        return Err(unauthorized(
            "Token exchange requires a caller token to exchange (OBO).",
            Some("Bearer error=\"invalid_request\""),
        ));
    };
    let token = exchanger
        .exchange(inbound, server, config, &subject.tenant_id)
        .await?;
    let (header_name, header_value) = config.header.header(token.access_token.expose_secret());
    let auth = StaticAuth::new(&header_name, &header_value)?;
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

pub(super) fn not_implemented(kind: AuthSpecKind) -> CredError {
    let message = format!("{kind}: resolver arm not implemented yet");
    CredError::NotImplemented(message)
}

// A store outage reads as no token, so the caller gets the OAuth challenge instead of a 503 it cannot fix by signing in.
async fn authz_token(
    subject: &Subject,
    server: &ServerSpec,
    store: &dyn OAuthTokenStore,
) -> Option<OAuthToken> {
    let fetched = store.fetch(&subject.subject_id, &server.server_id).await;
    match fetched {
        Ok(token) => token,
        Err(TokenStoreUnavailable) => None,
    }
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
