mod arms;
mod id_jag;
pub mod source;

use std::sync::Arc;

use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_spec::AuthSpecKind;
use crate::credential_contracts::errors::CredResult;
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credentials::ResolvedAuth;

use super::collaborators::{
    AssertionStore, ClientCredentialsSource, NullAssertionStore, NullClientCredentialsSource,
    NullTokenEndpoint, NullTokenExchanger, NullTokenStore, OAuthTokenStore, TokenEndpointClient,
    TokenExchanger,
};
use super::token_cache::ExchangedTokenCache;

pub struct Collaborators {
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
    pub fn with_oauth_token_store(mut self, store: Arc<dyn OAuthTokenStore>) -> Self {
        self.oauth_token_store = store;
        self
    }

    pub fn with_token_exchanger(mut self, exchanger: Arc<dyn TokenExchanger>) -> Self {
        self.token_exchanger = exchanger;
        self
    }

    pub fn with_token_endpoint(mut self, endpoint: Arc<dyn TokenEndpointClient>) -> Self {
        self.token_endpoint = endpoint;
        self
    }

    pub fn with_exchanged_tokens(mut self, cache: Arc<ExchangedTokenCache>) -> Self {
        self.exchanged_tokens = cache;
        self
    }

    pub fn with_client_credentials_source(
        mut self,
        source: Arc<dyn ClientCredentialsSource>,
    ) -> Self {
        self.client_credentials_source = source;
        self
    }

    pub fn with_sso_assertion_store(mut self, store: Arc<dyn AssertionStore>) -> Self {
        self.sso_assertion_store = store;
        self
    }
}

pub struct UpstreamCredentialProvider {
    collaborators: Collaborators,
}

impl UpstreamCredentialProvider {
    pub fn new(collaborators: Collaborators) -> Self {
        Self { collaborators }
    }

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
                arms::token_exchange(
                    subject,
                    server,
                    config,
                    &*self.collaborators.token_exchanger,
                )
                .await
            }
            AuthConfig::IdJag(config) => {
                arms::id_jag(
                    subject,
                    server,
                    config,
                    &self.collaborators,
                    chrono::Utc::now(),
                )
                .await
            }
            AuthConfig::AuthorizationCode(config) => {
                arms::authorization_code(
                    subject,
                    server,
                    config,
                    &*self.collaborators.oauth_token_store,
                )
                .await
            }
            AuthConfig::AwsSigv4(_config) => Err(arms::not_implemented(AuthSpecKind::AwsSigv4)),
        }
    }

    pub async fn has_user_token(&self, subject: &Subject, server: &ServerSpec) -> bool {
        arms::has_user_token(subject, server, &*self.collaborators.oauth_token_store).await
    }

    // Only id_jag and token_exchange hold a re-mintable credential here; client_credentials recovers inside its own auth flow and the rest mint nothing.
    pub async fn invalidate_credentials(&self, subject: &Subject, server: &ServerSpec) {
        match &server.config {
            AuthConfig::IdJag(_config) => {
                id_jag::invalidate(subject, server, &self.collaborators.exchanged_tokens)
            }
            AuthConfig::TokenExchange(config) => {
                let inbound = subject.inbound_token.as_ref();
                if let Some(token) = inbound {
                    self.collaborators
                        .token_exchanger
                        .invalidate(token, server, config, &subject.tenant_id)
                        .await;
                }
            }
            AuthConfig::AuthorizationCode(_config)
            | AuthConfig::ClientCredentials(_config)
            | AuthConfig::ApiKey(_config)
            | AuthConfig::Passthrough(_config)
            | AuthConfig::None(_config)
            | AuthConfig::AwsSigv4(_config) => {}
        }
    }
}
