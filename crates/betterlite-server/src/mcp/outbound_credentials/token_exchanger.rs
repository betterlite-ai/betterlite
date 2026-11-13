mod body;
mod cache_key;
mod form;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::oauth::{
    TokenExchangeConfig, TokenExchangeProfile,
};
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult, Unauthorized};
use betterlite_mcp::credential_contracts::server_spec::ServerSpec;
use betterlite_mcp::credential_contracts::token_endpoint_auth::build_token_client_auth;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use thiserror::Error;

use super::v2_store::CredentialValue;
use super::{OAuthToken, RefreshCoordinator, RefreshStep, TokenCacheBackend, TokenKey, WallClock};

pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);
pub const MIN_TTL: Duration = Duration::from_secs(10);
pub const EXPIRY_BUFFER: Duration = Duration::from_secs(60);

pub const TOKEN_EXCHANGE_GRANT: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
pub const ON_BEHALF_OF: &str = "on_behalf_of";

const NON_ACCESS_ISSUED_TOKEN_TYPES: [&str; 4] = [
    "urn:ietf:params:oauth:token-type:refresh_token",
    "urn:ietf:params:oauth:token-type:id_token",
    "urn:ietf:params:oauth:token-type:saml1",
    "urn:ietf:params:oauth:token-type:saml2",
];

pub type ExchangeBody = BTreeMap<String, CredentialValue>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExchangeRejection {
    #[error("the IdP rejected the subject token: {detail}")]
    SubjectRejected {
        detail: String,
        claims: Option<String>,
    },
    #[error("the IdP rejected the exchange request as {code}")]
    ClientFault { code: String },
}

// The IdP returns an opaque JSON object; the exchanger validates each field of it, so no untyped value passes this seam.
#[async_trait]
pub trait ExchangeHttpPost: Send + Sync {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, SecretString>,
        headers: &BTreeMap<String, SecretString>,
    ) -> Result<Option<ExchangeBody>, ExchangeRejection>;
}

pub struct OboTokenExchanger<Post, Cache, Coordinator> {
    pub post: Post,
    pub cache: Cache,
    pub coordinator: Coordinator,
    pub clock: Arc<dyn WallClock>,
    pub default_ttl: Duration,
    pub min_ttl: Duration,
    pub expiry_buffer: Duration,
}

// Fail closed before any request: no endpoint, no usable client credentials, or an Entra OBO without a target scope.
fn exchange_endpoint(config: &TokenExchangeConfig) -> CredResult<&str> {
    let configured = config
        .token_exchange_endpoint
        .as_deref()
        .filter(|value| !value.is_empty());
    let Some(endpoint) = configured else {
        return Err(CredError::PreconditionRequired(
            "token exchange endpoint is not configured for this server".to_owned(),
        ));
    };
    if config.client_id.as_deref().is_none_or(str::is_empty) || config.client_secret.is_none() {
        return Err(CredError::Misconfigured(
            "token_exchange requires client_id and client_secret".to_owned(),
        ));
    }
    if config.profile == TokenExchangeProfile::EntraObo && config.scopes.is_empty() {
        return Err(CredError::Misconfigured(
            "entra_obo token exchange requires a scope (e.g. api://<app-id>/.default)".to_owned(),
        ));
    }
    Ok(endpoint)
}

fn exchange_request(
    config: &TokenExchangeConfig,
    subject_token: &str,
) -> CredResult<(form::ExchangeForm, form::ExchangeForm)> {
    let client_auth = build_token_client_auth(
        config.token_endpoint_auth_method,
        config.client_id.as_deref(),
        config.client_secret.as_ref(),
    )
    .map_err(|error| CredError::Misconfigured(error.to_string()))?;
    let mut form = form::build_exchange_form(
        config.profile,
        subject_token,
        &config.subject_token_type,
        config.audience.as_deref(),
        &config.scopes,
    );
    form.extend(client_auth.body);
    Ok((form, client_auth.headers))
}

fn outcome(result: Result<Option<OAuthToken>, ExchangeRejection>) -> CredResult<OAuthToken> {
    match result {
        Ok(Some(token)) => Ok(token),
        Ok(None) => Err(CredError::UpstreamUnavailable(
            "token exchange did not return a usable access token".to_owned(),
        )),
        Err(ExchangeRejection::SubjectRejected { detail, claims }) => {
            Err(CredError::Unauthorized(Unauthorized {
                detail: if detail.is_empty() {
                    "subject token rejected by the IdP".to_owned()
                } else {
                    detail
                },
                www_authenticate: None,
                body: None,
                claims,
            }))
        }
        Err(ExchangeRejection::ClientFault { code }) => {
            tracing::warn!(%code, "MCP token exchange rejected the gateway's configuration");
            Err(CredError::Misconfigured(
                "token exchange configuration error: the gateway's credentials, audience, or \
                 scope for this server were not accepted by the IdP"
                    .to_owned(),
            ))
        }
    }
}

impl<Post, Cache, Coordinator> OboTokenExchanger<Post, Cache, Coordinator>
where
    Post: ExchangeHttpPost,
    Cache: TokenCacheBackend,
    Coordinator: RefreshCoordinator<Output = Option<OAuthToken>, Error = ExchangeRejection>,
{
    pub async fn exchange(
        &self,
        subject_token: &str,
        server: &ServerSpec,
        config: &TokenExchangeConfig,
        tenant_id: &str,
    ) -> CredResult<OAuthToken> {
        let endpoint = exchange_endpoint(config)?;
        let key = TokenKey {
            user_id: cache_key::cache_key(subject_token, tenant_id, config),
            server_id: server.server_id.clone(),
        };
        if let Some(hit) = self.cache.get(&key).await {
            tracing::debug!(server = %server.server_id, "MCP token exchange cache hit");
            return Ok(hit);
        }
        let (form, headers) = exchange_request(config, subject_token)?;
        let url = endpoint.to_owned();
        let refresh_key = key.clone();
        let refresh: RefreshStep<'_, Option<OAuthToken>, ExchangeRejection> = Box::new(move || {
            Box::pin(async move { self.refresh(&refresh_key, &url, &form, &headers).await })
        });
        let reread_key = key.clone();
        let reread: RefreshStep<'_, Option<OAuthToken>, ExchangeRejection> =
            Box::new(move || Box::pin(async move { Ok(self.cache.get(&reread_key).await) }));
        outcome(self.coordinator.run(key, refresh, reread).await)
    }

    pub async fn invalidate(
        &self,
        subject_token: &str,
        server: &ServerSpec,
        config: &TokenExchangeConfig,
        tenant_id: &str,
    ) {
        let key = TokenKey {
            user_id: cache_key::cache_key(subject_token, tenant_id, config),
            server_id: server.server_id.clone(),
        };
        self.cache.delete(&key).await;
    }

    // A re-entrant refresh on the same key reads the entry the first exchange stored instead of posting again.
    async fn refresh(
        &self,
        key: &TokenKey,
        endpoint: &str,
        form: &BTreeMap<String, SecretString>,
        headers: &BTreeMap<String, SecretString>,
    ) -> Result<Option<OAuthToken>, ExchangeRejection> {
        if let Some(fresh) = self.cache.get(key).await {
            return Ok(Some(fresh));
        }
        tracing::debug!(server = %key.server_id, "MCP token exchange started");
        let Some(body) = self.post.post(endpoint, form, headers).await? else {
            return Ok(None);
        };
        let Some(token) = body::token_from_body(&body, self.clock.now()) else {
            return Ok(None);
        };
        let ttl = self.ttl(&token, self.clock.now());
        self.cache.set(key, &token, ttl).await;
        tracing::info!(server = %key.server_id, "MCP token exchange succeeded");
        Ok(Some(token))
    }

    fn ttl(&self, token: &OAuthToken, now: DateTime<Utc>) -> Duration {
        let Some(expires_at) = token.expires_at else {
            return self.default_ttl;
        };
        let lifetime = expires_at
            .signed_duration_since(now)
            .to_std()
            .unwrap_or_default();
        lifetime
            .saturating_sub(self.expiry_buffer)
            .max(self.min_ttl)
            .min(lifetime)
    }
}
