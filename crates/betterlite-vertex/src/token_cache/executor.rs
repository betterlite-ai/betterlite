use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

use super::error::CredentialCacheError;
use super::record::CredentialSnapshot;

// what load_auth resolves: the credential state to cache and the project it resolved
#[derive(Debug, Clone)]
pub struct LoadedCredential {
    pub credential: Option<Arc<CredentialSnapshot>>,
    pub project: Option<String>,
}

// VertexBase's load_auth and refresh_auth, injected so the cache owns no SDK handle and parses no response
pub trait CredentialExecutor: Send + Sync {
    fn load<'borrow>(
        &'borrow self,
        credentials: Option<&'borrow ConfiguredCredentials>,
        project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    >;

    fn refresh<'borrow>(
        &'borrow self,
        credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    >;
}
