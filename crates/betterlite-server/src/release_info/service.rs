use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, Method};
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use thiserror::Error;
use tokio::sync::Mutex;
use url::Url;

use super::records::{LatestRelease, LatestReleaseUnavailable, decode_latest_release};
use crate::roi_github::transport::Transport;

pub(super) const LATEST_RELEASE_CACHE_KEY: &str = "latest_release_info";
const CACHE_CAPACITY: i64 = 1;
const SUCCESS_TTL_SECONDS: f64 = 3_600.0;
const UNAVAILABLE_TTL_SECONDS: f64 = 300.0;
const ITEM_KIB: i64 = 1_024;
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub(super) enum ReleaseError {
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error("the latest release source is not a URL")]
    Source(#[source] Box<url::ParseError>),
    #[error("the latest release entry does not match its wire shape")]
    Shape(#[source] Box<serde_json::Error>),
}

pub(super) struct ReleaseService<Client> {
    client: Client,
    source: String,
    clock: Clock,
    cache: Mutex<Option<LocalTtlCache>>,
    fetch_lock: Mutex<()>,
}

impl<Client> ReleaseService<Client>
where
    Client: Transport,
{
    pub(super) fn new(client: Client, source: String, clock: Clock) -> Self {
        Self {
            client,
            source,
            clock,
            cache: Mutex::new(None),
            fetch_lock: Mutex::new(()),
        }
    }

    pub(super) async fn get(&self) -> Result<LatestRelease, ReleaseError> {
        if let Some(cached) = self.cached().await? {
            return Ok(cached);
        }
        let _fetching = self.fetch_lock.lock().await;
        if let Some(cached) = self.cached().await? {
            return Ok(cached);
        }
        // A caller that goes away drops this future before the store, so a cancellation is never cached.
        let release = self.fetch().await?;
        self.store(&release).await?;
        Ok(release)
    }

    async fn cached(&self) -> Result<Option<LatestRelease>, ReleaseError> {
        let entry = self
            .with_cache(|cache| cache.get(LATEST_RELEASE_CACHE_KEY))
            .await?;
        let Some(CacheValue::Array(items)) = entry else {
            return Ok(None);
        };
        let Some(CacheValue::Text(text)) = items.first() else {
            return Ok(None);
        };
        let release: LatestRelease =
            serde_json::from_str(text).map_err(|error| ReleaseError::Shape(Box::new(error)))?;
        Ok(Some(release))
    }

    async fn store(&self, release: &LatestRelease) -> Result<(), ReleaseError> {
        let ttl = match release {
            LatestRelease::Available(_) => SUCCESS_TTL_SECONDS,
            LatestRelease::Unavailable(_) => UNAVAILABLE_TTL_SECONDS,
        };
        let text =
            serde_json::to_string(release).map_err(|error| ReleaseError::Shape(Box::new(error)))?;
        let value = CacheValue::Array(vec![CacheValue::Text(text)]);
        self.with_cache(|cache| cache.set(LATEST_RELEASE_CACHE_KEY, value, Some(ttl)))
            .await
    }

    async fn fetch(&self) -> Result<LatestRelease, ReleaseError> {
        let url =
            Url::parse(&self.source).map_err(|error| ReleaseError::Source(Box::new(error)))?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: RELEASE_TIMEOUT,
        };
        match self.client.send(request).await {
            Ok(response) => Ok(decode_latest_release(response.status, &response.body)),
            Err(error) => Ok(LatestRelease::Unavailable(Box::new(
                LatestReleaseUnavailable {
                    reason: error.to_string(),
                },
            ))),
        }
    }

    async fn with_cache<Output, Action>(&self, action: Action) -> Result<Output, ReleaseError>
    where
        Action: FnOnce(&mut LocalTtlCache) -> Result<Output, CacheError>,
    {
        let mut slot = self.cache.lock().await;
        if slot.is_none() {
            let built = LocalTtlCache::with_clock(
                Some(CACHE_CAPACITY),
                Some(SUCCESS_TTL_SECONDS),
                Some(ITEM_KIB),
                ITEM_KIB,
                Arc::clone(&self.clock),
            )?;
            *slot = Some(built);
        }
        let outcome = slot.as_mut().map_or_else(
            || Err(ReleaseError::Cache(CacheError::Unavailable)),
            |cache| action(cache).map_err(ReleaseError::Cache),
        );
        drop(slot);
        outcome
    }
}
