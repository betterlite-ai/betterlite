pub mod error;
mod tests;

use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_guardrails::model_armor::transport::tokens::ArmorTokens;
use betterlite_vertex::credential_sources::error::CredentialSourceError;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::model_policy::capabilities::VertexCapabilityProvider;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::executor::CredentialExecutor;
use betterlite_vertex::token_refresh::{CredentialRefresh, ensure_access_token};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;

use error::ArmorTokenError;

// VertexBase.__init__ runs once per ModelArmorGuardrail, so each source keeps its own cache and token
pub struct VertexArmorTokens {
    credentials: Option<ConfiguredCredentials>,
    project_id: Option<String>,
    refresh: CredentialRefresh,
}

impl VertexArmorTokens {
    pub fn new(
        credentials: Option<SecretString>,
        project_id: Option<String>,
        clock: Arc<dyn CredentialClock>,
        executor: Arc<dyn CredentialExecutor>,
        tasks: TaskTracker,
    ) -> Result<Self, ArmorTokenError> {
        // B1301 owns the "Unable to load vertex credentials" text, so a decode failure wears its error
        let configured = credentials
            .map(|secret| ConfiguredCredentials::from_text(secret.expose_secret().to_owned()))
            .transpose()
            .map_err(|source| CredentialSourceError::Credentials { source })?;
        let cache = Arc::new(CredentialCache::new(clock, executor));
        Ok(Self {
            credentials: configured,
            project_id,
            refresh: CredentialRefresh::new(cache, tasks),
        })
    }
}

impl Debug for VertexArmorTokens {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("VertexArmorTokens")
            .field("credentials", &self.credentials.is_some())
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ArmorTokens for VertexArmorTokens {
    type Error = ArmorTokenError;

    async fn access_token(&self) -> Result<(SecretString, Option<String>), ArmorTokenError> {
        // custom_llm_provider is the literal vertex_ai make_model_armor_request passes
        let access = ensure_access_token(
            VertexCapabilityProvider::VertexAi,
            self.refresh
                .get_access_token_async(self.credentials.as_ref(), self.project_id.as_deref()),
        )
        .await?;
        Ok((access.token, Some(access.project)))
    }
}
