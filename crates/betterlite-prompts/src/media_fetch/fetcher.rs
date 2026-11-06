use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use thiserror::Error;
use tokio::time::timeout;

use super::cache::ImageCache;
use super::error::ImageFetchError;
use super::response::{BodyReadError, ImageResponse, ProcessError, process_image_response};

const MAX_CONCURRENT_MEDIA_FETCHES: usize = 20;
const ATTEMPTS: usize = 3;

pub struct Fetcher<R> {
    client: ProxiedClient,
    urls: UserUrlFetcher<R>,
    max_download_size_mb: f64,
    cache: ImageCache,
}

impl<R> Fetcher<R> {
    pub fn new(
        client: ProxiedClient,
        urls: UserUrlFetcher<R>,
        max_download_size_mb: f64,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ) -> Result<Self, CacheError> {
        let cache = ImageCache::new(clock)?;
        Ok(Self {
            client,
            urls,
            max_download_size_mb,
            cache,
        })
    }
}

#[derive(Debug, Error)]
enum AttemptError {
    #[error(transparent)]
    Url(#[from] UserUrlError),
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("the image fetch exceeded its deadline")]
    Deadline,
}

enum AttemptFailure {
    Failed(ImageFetchError),
    Retry(AttemptError),
}

impl<R> Fetcher<R>
where
    R: Resolve,
{
    pub async fn convert(&self, url: &str) -> Result<String, ImageFetchError> {
        if url.starts_with("data:") && url.contains(";base64,") {
            return Ok(url.to_owned());
        }
        if self.max_download_size_mb == 0.0_f64 {
            return Err(ImageFetchError::Disabled {
                url: url.to_owned(),
            });
        }
        if let Some(cached) = self.cache.get(url) {
            return Ok(cached);
        }
        for _ in 0..ATTEMPTS {
            match self.attempt(url).await {
                Ok(data_url) => return Ok(data_url),
                Err(AttemptFailure::Failed(error)) => return Err(error),
                Err(AttemptFailure::Retry(error)) => {
                    tracing::error!(error = %error, url = %url, "an image fetch attempt failed");
                }
            }
        }
        Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        })
    }

    pub async fn fetch_all(&self, urls: &[String]) -> Result<Vec<String>, ImageFetchError> {
        let mut fetches = stream::iter(urls.iter().enumerate())
            .map(|(index, url)| async move { (index, self.convert(url).await) })
            .buffer_unordered(MAX_CONCURRENT_MEDIA_FETCHES);
        let mut results = Vec::with_capacity(urls.len());
        while let Some((index, result)) = fetches.next().await {
            results.push((index, result?));
        }
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, data_url)| data_url).collect())
    }

    // Each try gets LiteLLM's request timeout as a total deadline on top of the client's per-read timeout
    async fn attempt(&self, url: &str) -> Result<String, AttemptFailure> {
        let deadline = self.client.limits.read_timeout;
        // Tokio's timer rounds a zero duration up to the next millisecond, so the failing deadline is decided before the fetch
        if deadline.is_zero() {
            return Err(AttemptFailure::Retry(AttemptError::Deadline));
        }
        let Ok(result) = timeout(deadline, self.fetch_and_process(url)).await else {
            return Err(AttemptFailure::Retry(AttemptError::Deadline));
        };
        result
    }

    async fn fetch_and_process(&self, url: &str) -> Result<String, AttemptFailure> {
        let headers = HeaderMap::new();
        let fetched = match self.urls.fetch(&self.client, url, &headers).await {
            Ok(fetched) => fetched,
            Err(UserUrlError::Ssrf(error)) => {
                tracing::warn!(
                    "Image fetch of {url} rejected before any request went out: {error}"
                );
                return Err(AttemptFailure::Failed(ImageFetchError::Rejected {
                    url: url.to_owned(),
                }));
            }
            Err(error) => return Err(AttemptFailure::Retry(AttemptError::Url(error))),
        };
        let response = ImageResponse {
            status: fetched.response.status().as_u16(),
            headers: fetched.response.headers().clone(),
            body: fetched.body_stream(),
        };
        match process_image_response(response, url, self.max_download_size_mb, &self.cache).await {
            Ok(data_url) => Ok(data_url),
            Err(ProcessError::Image(error)) => Err(AttemptFailure::Failed(error)),
            Err(ProcessError::Body(BodyReadError::TooLarge(error))) => {
                Err(AttemptFailure::Failed(error))
            }
            Err(error) => Err(AttemptFailure::Retry(AttemptError::Process(error))),
        }
    }
}
