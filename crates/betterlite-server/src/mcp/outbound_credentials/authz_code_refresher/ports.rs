use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_db::oauth_token_resolution::ports::TokenResponse;
use betterlite_mcp::server_record::MCPServer;

use super::TokenStoreUnavailable;

// LiteLLM's ServerLookup: get_mcp_server_by_id, read once per refresh.
pub trait ServerLookup: Send + Sync {
    fn server(&self, server_id: &str) -> Option<Arc<MCPServer>>;
}

// LiteLLM's TokenEndpointPost: the parsed JSON answer, or None for any transport, status or JSON failure.
pub trait TokenEndpointPost: Send + Sync {
    fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> impl Future<Output = Option<TokenResponse>> + Send;
}

pub struct PersistRequest<'request> {
    pub user_id: &'request str,
    pub server_id: &'request str,
    pub access_token: &'request str,
    pub refresh_token: Option<&'request str>,
    pub expires_in: Option<i64>,
    pub scopes: Option<&'request [String]>,
    pub identity_binding_proof: Option<&'request str>,
}

impl fmt::Debug for PersistRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistRequest")
            .field("user_id", &self.user_id)
            .field("server_id", &self.server_id)
            .field("access_token", &"***")
            .field("has_refresh_token", &self.refresh_token.is_some())
            .field("expires_in", &self.expires_in)
            .field("scopes", &self.scopes)
            .field("identity_binding_proof", &self.identity_binding_proof)
            .finish()
    }
}

pub trait CredentialPersist: Send + Sync {
    fn persist(
        &self,
        request: &PersistRequest<'_>,
    ) -> impl Future<Output = Result<(), TokenStoreUnavailable>> + Send;
}

// LiteLLM's clock: time.time() as epoch seconds, so one call shapes every expires_at.
pub trait EpochClock: Send + Sync {
    fn now(&self) -> f64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemEpochClock;

impl EpochClock for SystemEpochClock {
    fn now(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

impl<T: ServerLookup + ?Sized> ServerLookup for Arc<T> {
    fn server(&self, server_id: &str) -> Option<Arc<MCPServer>> {
        (**self).server(server_id)
    }
}

impl<T: TokenEndpointPost + ?Sized> TokenEndpointPost for Arc<T> {
    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> Option<TokenResponse> {
        (**self).post_form(url, form, headers).await
    }
}

impl<T: CredentialPersist + ?Sized> CredentialPersist for Arc<T> {
    async fn persist(&self, request: &PersistRequest<'_>) -> Result<(), TokenStoreUnavailable> {
        (**self).persist(request).await
    }
}
