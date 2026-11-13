use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::SecretString;

use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

pub type MintFuture<'mint> =
    Pin<Box<dyn Future<Output = Result<AccessToken, CredentialCacheError>> + Send + 'mint>>;

// LiteLLM's injected mint_access_token: it resolves an access token and the project the target is built from
pub trait MintAccessToken: Send + Sync {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint>;
}

// the replacement LiteLLM installs by default, VertexBase._ensure_access_token through the credential cache
pub struct CredentialMint {
    pub cache: Arc<CredentialCache>,
}

impl MintAccessToken for CredentialMint {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(self.cache.get_access_token(credentials, project))
    }
}

// Lyria builds an interactions URL with a configured project, and no credential refresh may run for it
pub struct ProjectOnlyMint {
    pub project: Option<String>,
}

impl MintAccessToken for ProjectOnlyMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        let resolved = project
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| self.project.clone())
            .unwrap_or_default();
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from(""),
            project: resolved,
        })))
    }
}
