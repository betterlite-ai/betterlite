use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::anthropic_beta_config::Settings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::HeaderMap;
use indexmap::IndexMap;
use tokio::sync::{Mutex, MutexGuard};
use url::Url;

use crate::api::api_boundaries::ApiLimits;
use error::BetaConfigError;
use matrix::{BetaMatrix, BetaNode, decode_beta_matrix, validate_beta_matrix};

pub mod error;
pub mod matrix;
mod tests;

const BUNDLED_CONFIG: &str =
    include_str!("anthropic_beta_config/anthropic_beta_headers_config.json");
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const EMPTY_PROVIDERS: [&str; 6] = [
    "anthropic",
    "azure_ai",
    "bedrock",
    "bedrock_converse",
    "vertex_ai",
    "provider_aliases",
];

// The process-local beta matrix: loaded on first use, cached even when it is the local fallback, and replaced only by reload.
pub struct BetaConfigLoader {
    client: ProxiedClient,
    settings: Settings,
    limits: ApiLimits,
    bundled: &'static str,
    cache: Mutex<Option<Arc<BetaMatrix>>>,
}

impl BetaConfigLoader {
    #[must_use]
    pub const fn new(client: ProxiedClient, settings: Settings, limits: ApiLimits) -> Self {
        Self {
            client,
            settings,
            limits,
            bundled: BUNDLED_CONFIG,
            cache: Mutex::const_new(None),
        }
    }

    pub async fn matrix(&self) -> Arc<BetaMatrix> {
        let cache = self.cache.lock().await;
        if let Some(loaded) = cache.as_ref() {
            return Arc::clone(loaded);
        }
        self.fill(cache).await
    }

    pub async fn reload(&self) -> Arc<BetaMatrix> {
        let mut cache = self.cache.lock().await;
        *cache = None;
        tracing::info!(
            kind = "reloading",
            "Reloading beta headers config (cache cleared)"
        );
        self.fill(cache).await
    }

    async fn fill(&self, mut cache: MutexGuard<'_, Option<Arc<BetaMatrix>>>) -> Arc<BetaMatrix> {
        let loaded = Arc::new(self.load().await);
        tracing::debug!(
            kind = "cache_filled",
            "Loaded and cached beta headers config"
        );
        *cache = Some(Arc::clone(&loaded));
        drop(cache);
        loaded
    }

    async fn load(&self) -> BetaMatrix {
        if self.settings.local_only {
            return self.local();
        }
        let url = &self.settings.url;
        let node = match self.fetch().await {
            Ok(node) => node,
            Err(error) => {
                tracing::warn!(
                    kind = "fetch_failed",
                    url,
                    ?error,
                    "Failed to fetch remote beta headers config. Falling back to local backup."
                );
                return self.local();
            }
        };
        match validate_beta_matrix(node) {
            Ok(matrix) => matrix,
            Err(shape) => {
                tracing::warn!(
                    kind = shape.kind(),
                    ?shape,
                    "Fetched beta headers config is invalid. Falling back to local backup."
                );
                tracing::warn!(
                    kind = "integrity_failed",
                    url,
                    "Fetched beta headers config failed integrity check. Using local backup instead."
                );
                self.local()
            }
        }
    }

    async fn fetch(&self) -> Result<BetaNode, BetaConfigError> {
        let request = OutboundRequest {
            method: Method::GET,
            url: Url::parse(&self.settings.url).map_err(BetaConfigError::Url)?,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: FETCH_TIMEOUT,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(BetaConfigError::Request)?;
        if !(200..300).contains(&response.status) {
            return Err(BetaConfigError::Status {
                status: response.status,
            });
        }
        decode_beta_matrix(&response.body, self.limits)
    }

    fn local(&self) -> BetaMatrix {
        match serde_json::from_str(self.bundled) {
            Ok(matrix) => matrix,
            Err(source) => {
                let error = BetaConfigError::Bundled(source);
                tracing::error!(
                    kind = "local_load_failed",
                    ?error,
                    "Failed to load local beta headers config"
                );
                BetaMatrix(
                    EMPTY_PROVIDERS
                        .iter()
                        .map(|name| ((*name).to_owned(), BetaNode::Map(IndexMap::new())))
                        .collect(),
                )
            }
        }
    }
}
