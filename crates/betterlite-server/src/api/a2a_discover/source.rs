use std::time::Duration;

use async_trait::async_trait;
use axum::http::HeaderMap;
use betterlite_core::agents::json::AgentJson;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::SystemResolver;
use tokio::time::timeout;

use super::discovery::{CardResponse, CardSource, CardTransportError, fetch_well_known_card};
use super::{DiscoverAgentCard, DiscoverAgentRequest, DiscoveryFailure};

// The SSRF-safe fetch LiteLLM's async_safe_get wraps, with a total deadline on each attempt
pub struct SafeCardSource<'source> {
    pub client: &'source ProxiedClient,
    pub urls: &'source UserUrlFetcher<SystemResolver>,
    pub timeout: Duration,
}

#[async_trait]
impl CardSource for SafeCardSource<'_> {
    async fn get(&self, url: &str) -> Result<CardResponse, CardTransportError> {
        let headers = HeaderMap::new();
        let fetched = match timeout(self.timeout, self.urls.fetch(self.client, url, &headers)).await
        {
            Ok(result) => result?,
            Err(_) => {
                return Err(CardTransportError::Deadline {
                    timeout: self.timeout,
                });
            }
        };
        let response = fetched.buffered(self.client.limits.body_bytes).await?;
        Ok(CardResponse {
            status: response.status,
            body: response.body,
        })
    }
}

pub struct LiveDiscovery {
    pub client: ProxiedClient,
    pub urls: UserUrlFetcher<SystemResolver>,
    pub timeout: Duration,
}

#[async_trait]
impl DiscoverAgentCard for LiveDiscovery {
    async fn discover(
        &self,
        request: &DiscoverAgentRequest,
    ) -> Result<AgentJson, DiscoveryFailure> {
        let source = SafeCardSource {
            client: &self.client,
            urls: &self.urls,
            timeout: self.timeout,
        };
        match fetch_well_known_card(
            &source,
            &request.url,
            request.discovery_mode,
            request.params.as_ref(),
        )
        .await
        {
            Ok(card) => Ok(card),
            Err(error) => Err(DiscoveryFailure::Card(error)),
        }
    }
}
