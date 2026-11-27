pub mod documents;
pub mod routes;
#[cfg(test)]
mod tests;
pub mod upstream_fetch;

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::extract::Extension;
use axum::routing::get;
use betterlite_mcp::idp_token_exchange::TokenExchangePrerequisites;
use betterlite_mcp::oauth_metadata_cache::OAuthMetadataCache;
use betterlite_mcp::registry::servers::SharedRegistry;
use betterlite_models::decisions::Json;
use http::request::Parts;
use indexmap::IndexMap;
use thiserror::Error;
use tokio::time::Instant;

use self::routes::{
    AUTHORIZATION_SERVER_PATH, NamedShape, PROTECTED_RESOURCE_PATH, aggregate_authorization_server,
    aggregate_resource, named_document, root_document,
};

pub type JsonObject = IndexMap<String, Json>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpstreamReply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum GetFailure {
    #[error("{0}")]
    Transport(String),
    #[error("{0}")]
    Other(String),
}

pub trait UpstreamGet: Send + Sync {
    fn get(
        &self,
        url: &str,
        accept: &str,
    ) -> impl Future<Output = Result<UpstreamReply, GetFailure>> + Send;
}

pub trait RequestFacts: Send + Sync {
    fn request_base_url(&self, request: &Parts) -> String;

    fn client_ip(&self, request: &Parts) -> Option<String>;
}

pub trait GatewayFacts: Send + Sync {
    fn jwt_issuers(&self) -> Vec<String>;

    fn token_exchange(&self) -> TokenExchangePrerequisites;
}

pub struct ProtectedRoot<U, R, G> {
    pub registry: SharedRegistry,
    pub cache: OAuthMetadataCache,
    pub upstream: U,
    pub request: R,
    pub gateway: G,
    pub well_known_suffix: String,
    pub clock: fn() -> Instant,
}

impl<U, R, G> ProtectedRoot<U, R, G>
where
    U: UpstreamGet + 'static,
    R: RequestFacts + 'static,
    G: GatewayFacts + 'static,
{
    #[must_use]
    pub fn router(self) -> Router {
        let suffix = self.well_known_suffix.as_str();
        let aggregate_resource_path = format!("{PROTECTED_RESOURCE_PATH}{suffix}/mcp");
        let standard_path = format!("{PROTECTED_RESOURCE_PATH}{suffix}/mcp/{{mcp_server_name}}");
        let legacy_path = format!("{PROTECTED_RESOURCE_PATH}{suffix}/{{mcp_server_name}}/mcp");
        let aggregate_server_path = format!("{AUTHORIZATION_SERVER_PATH}{suffix}/mcp");
        Router::new()
            .route(PROTECTED_RESOURCE_PATH, get(root_document::<U, R, G>))
            .route(&aggregate_resource_path, get(aggregate_resource::<U, R, G>))
            .route(
                &aggregate_server_path,
                get(aggregate_authorization_server::<U, R, G>),
            )
            .route(
                &standard_path,
                get(named_document::<U, R, G>).layer(Extension(NamedShape::Standard)),
            )
            .route(
                &legacy_path,
                get(named_document::<U, R, G>).layer(Extension(NamedShape::Legacy)),
            )
            .with_state(Arc::new(self))
    }
}
