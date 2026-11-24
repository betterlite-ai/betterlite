mod arms;
mod id_jag;
pub mod source;

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_spec::AuthSpecKind;
use crate::credential_contracts::errors::{CredError, CredResult};
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credentials::ResolvedAuth;

use super::collaborators::{
    AssertionStore, ClientCredentialsSource, ExchangeRequest, NullAssertionStore,
    NullClientCredentialsSource, NullTokenEndpoint, NullTokenExchanger, NullTokenStore,
    OAuthTokenStore, TokenEndpointClient, TokenExchanger,
};
use super::token_cache::ExchangedTokenCache;

pub trait WallClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct Collaborators {
    pub wall_clock: Arc<dyn WallClock>,
    pub oauth_token_store: Arc<dyn OAuthTokenStore>,
    pub token_exchanger: Arc<dyn TokenExchanger>,
    pub token_endpoint: Arc<dyn TokenEndpointClient>,
    pub exchanged_tokens: Arc<ExchangedTokenCache>,
    pub client_credentials_source: Arc<dyn ClientCredentialsSource>,
    pub sso_assertion_store: Arc<dyn AssertionStore>,
}

impl Default for Collaborators {
    fn default() -> Self {
        Self {
            wall_clock: Arc::new(SystemWallClock),
            oauth_token_store: Arc::new(NullTokenStore),
            token_exchanger: Arc::new(NullTokenExchanger),
            token_endpoint: Arc::new(NullTokenEndpoint),
            exchanged_tokens: Arc::new(ExchangedTokenCache::default()),
            client_credentials_source: Arc::new(NullClientCredentialsSource),
            sso_assertion_store: Arc::new(NullAssertionStore),
        }
    }
}

impl Collaborators {
    #[must_use]
    pub fn with_wall_clock(mut self, clock: Arc<dyn WallClock>) -> Self {
        self.wall_clock = clock;
        self
    }

    #[must_use]
    pub fn with_oauth_token_store(mut self, store: Arc<dyn OAuthTokenStore>) -> Self {
        self.oauth_token_store = store;
        self
    }

    #[must_use]
    pub fn with_token_exchanger(mut self, exchanger: Arc<dyn TokenExchanger>) -> Self {
        self.token_exchanger = exchanger;
        self
    }

    #[must_use]
    pub fn with_token_endpoint(mut self, endpoint: Arc<dyn TokenEndpointClient>) -> Self {
        self.token_endpoint = endpoint;
        self
    }

    #[must_use]
    pub fn with_exchanged_tokens(mut self, cache: Arc<ExchangedTokenCache>) -> Self {
        self.exchanged_tokens = cache;
        self
    }

    #[must_use]
    pub fn with_client_credentials_source(
        mut self,
        source: Arc<dyn ClientCredentialsSource>,
    ) -> Self {
        self.client_credentials_source = source;
        self
    }

    #[must_use]
    pub fn with_sso_assertion_store(mut self, store: Arc<dyn AssertionStore>) -> Self {
        self.sso_assertion_store = store;
        self
    }
}

pub struct UpstreamCredentialProvider {
    pub collaborators: Collaborators,
}

impl UpstreamCredentialProvider {
    pub async fn resolve_credentials(
        &self,
        subject: &Subject,
        server: &ServerSpec,
    ) -> CredResult<Arc<dyn ResolvedAuth>> {
        match &server.config {
            AuthConfig::None(_config) => arms::no_credential(server),
            AuthConfig::ApiKey(config) => arms::api_key(config),
            AuthConfig::Passthrough(_config) => arms::passthrough(subject),
            AuthConfig::ClientCredentials(config) => {
                arms::client_credentials(
                    &server.server_id,
                    config,
                    &self.collaborators.client_credentials_source,
                )
                .await
            }
            AuthConfig::TokenExchange(config) => {
                let caller =
                    arms::CallerToken::Exchanged(&*self.collaborators.token_exchanger, config);
                arms::user_token(subject, server, &config.header, caller).await
            }
            AuthConfig::IdJag(config) => {
                let now = self.collaborators.wall_clock.now();
                arms::id_jag(subject, server, config, &self.collaborators, now).await
            }
            AuthConfig::AuthorizationCode(config) => {
                let caller = arms::CallerToken::Stored(&*self.collaborators.oauth_token_store);
                arms::user_token(subject, server, &config.header, caller).await
            }
            AuthConfig::AwsSigv4(_config) => Err(CredError::NotImplemented(format!(
                "{}: resolver arm not implemented yet",
                AuthSpecKind::AwsSigv4
            ))),
        }
    }

    #[must_use]
    pub async fn has_user_token(&self, subject: &Subject, server: &ServerSpec) -> bool {
        let store: &dyn OAuthTokenStore = &*self.collaborators.oauth_token_store;
        let fetched = arms::authz_token(subject, server, store).await;
        fetched.is_some()
    }

    // Only id_jag and token_exchange hold a re-mintable credential here; client_credentials recovers inside its own auth flow and the rest mint nothing.
    pub async fn invalidate_credentials(&self, subject: &Subject, server: &ServerSpec) {
        if let AuthConfig::IdJag(_config) = &server.config {
            let slot = id_jag::slot_key(subject, server);
            self.collaborators.exchanged_tokens.invalidate(&slot);
        }
        if let AuthConfig::TokenExchange(config) = &server.config
            && let Some(token) = &subject.inbound_token
        {
            let request = ExchangeRequest {
                subject_token: token,
                server,
                config,
                tenant_id: &subject.tenant_id,
            };
            self.collaborators.token_exchanger.invalidate(request).await;
        }
    }
}
