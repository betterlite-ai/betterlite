use std::future::{Future, ready};
use std::pin::Pin;

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

// LiteLLM's default mint, VertexBase._ensure_access_token, is the credential cache's lookup
impl MintAccessToken for CredentialCache {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(self.get_access_token(credentials, project))
    }
}

// Lyria already resolved its project, and building the URL must not refresh a credential
pub struct ProjectOnlyMint {
    pub project: String,
}

impl MintAccessToken for ProjectOnlyMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        let resolved = project
            .filter(|value| !value.is_empty())
            .map_or_else(|| self.project.clone(), str::to_owned);
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from(""),
            project: resolved,
        })))
    }
}
