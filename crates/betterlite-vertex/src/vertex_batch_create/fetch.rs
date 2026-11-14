use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use http::header::HeaderMap;
use tokio::time::timeout;

use super::error::VertexBatchCreateError;

pub trait ProtectedGet: Send + Sync {
    fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        deadline: Duration,
    ) -> impl Future<Output = Result<UpstreamResponse, VertexBatchCreateError>> + Send;
}

// LiteLLM wraps the lookup in safe_get, so the bearer token never reaches a blocked address
pub struct SafeGet<'urls, Resolver> {
    pub urls: &'urls UserUrlFetcher<Resolver>,
    pub client: &'urls ProxiedClient,
}

impl<Resolver> ProtectedGet for SafeGet<'_, Resolver>
where
    Resolver: Resolve,
{
    async fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        deadline: Duration,
    ) -> Result<UpstreamResponse, VertexBatchCreateError> {
        let attempt = async {
            let fetched = self
                .urls
                .fetch(self.client, url, headers)
                .await
                .map_err(|source| VertexBatchCreateError::Lookup { source })?;
            fetched
                .buffered(self.client.limits.body_bytes)
                .await
                .map_err(VertexBatchCreateError::from_transport)
        };
        // safe_get returns at the response head, so the deadline has to cover the body read too
        timeout(deadline, attempt)
            .await
            .map_err(|_elapsed| VertexBatchCreateError::LookupTimeout)?
    }
}
