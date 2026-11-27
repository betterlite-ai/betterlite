use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::rag::vertex_ingestion::ports::{PortError, VertexTokenPort};
use betterlite_vertex::token_cache::CredentialCache;
use secrecy::SecretString;

mod tests;

// B3455's token port over the shared Vertex credential cache: explicit credentials win, otherwise ADC
pub struct VertexTokenSource {
    pub cache: Arc<CredentialCache>,
}

impl Debug for VertexTokenSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("VertexTokenSource").finish_non_exhaustive()
    }
}

#[async_trait]
impl VertexTokenPort for VertexTokenSource {
    async fn access_token(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
    ) -> Result<(SecretString, String), PortError> {
        let access = self
            .cache
            .get_access_token(credentials, project)
            .await
            .map_err(PortError::new)?;
        Ok((access.token, access.project))
    }
}
