use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;

pub(crate) type Prefetched = BTreeMap<String, Result<String, Arc<ImageFetchError>>>;

// LiteLLM fetches each remote file while it walks the messages, so a failure stops the walk where it is.
pub(crate) async fn prefetch<Source>(fetcher: &Source, messages: &[Json]) -> Prefetched
where
    Source: RemoteMediaFetcher,
{
    let mut fetched = Prefetched::new();
    for url in http_file_urls(messages) {
        if fetched.contains_key(&url) {
            continue;
        }
        let fetched_url = fetcher.convert_url(&url).await;
        let failed = fetched_url.is_err();
        fetched.insert(url, fetched_url.map_err(Arc::new));
        if failed {
            break;
        }
    }
    fetched
}

// The prompt assembly reads a fetched file from here, so the same URL never leaves the proxy twice.
pub(crate) struct Files<'a, Source> {
    pub(crate) fetcher: &'a Source,
    pub(crate) fetched: &'a Prefetched,
}

impl<Source> Files<'_, Source>
where
    Source: RemoteMediaFetcher + Sync,
{
    // A prefetched answer wins over the fetcher, so the same URL never leaves the proxy twice.
    async fn resolve(&self, url: &str) -> Result<String, ImageFetchError> {
        match self.fetched.get(url) {
            Some(Ok(fetched)) => Ok(fetched.clone()),
            Some(Err(error)) => Err(reraise(error)),
            None => self.fetcher.convert_url(url).await,
        }
    }
}

impl<Source> RemoteMediaFetcher for Files<'_, Source>
where
    Source: RemoteMediaFetcher + Sync,
{
    fn convert_url(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<String, ImageFetchError>> + Send {
        async move { self.resolve(url).await }
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> + Send {
        async move {
            let mut converted = Vec::with_capacity(urls.len());
            for url in urls {
                converted.push(self.resolve(url).await?);
            }
            Ok(converted)
        }
    }
}

// ImageFetchError carries no Clone, so the stored failure is rebuilt with the message it raised.
fn reraise(error: &ImageFetchError) -> ImageFetchError {
    match error {
        ImageFetchError::Disabled { url } => ImageFetchError::Disabled { url: url.clone() },
        ImageFetchError::Rejected { url } => ImageFetchError::Rejected { url: url.clone() },
        ImageFetchError::Attempts { url } => ImageFetchError::Attempts { url: url.clone() },
        ImageFetchError::Status { status, url } => ImageFetchError::Status {
            status: *status,
            url: url.clone(),
        },
        ImageFetchError::TooLarge { size, limit, url } => ImageFetchError::TooLarge {
            size: size.clone(),
            limit: limit.clone(),
            url: url.clone(),
        },
        ImageFetchError::UnknownMediaType { url, source } => ImageFetchError::UnknownMediaType {
            url: url.clone(),
            source: source.clone(),
        },
    }
}

fn http_file_urls(messages: &[Json]) -> Vec<String> {
    let mut urls = Vec::new();
    for message in messages {
        let Json::Object(fields) = message else {
            continue;
        };
        let Some(Json::List(parts)) = fields.get("content") else {
            continue;
        };
        for part in parts {
            let Json::Object(fields) = part else {
                continue;
            };
            if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "file") {
                continue;
            }
            let Some(Json::Object(file)) = fields.get("file") else {
                continue;
            };
            let Some(Json::Text(data)) = file.get("file_data") else {
                continue;
            };
            if data.starts_with("http") {
                urls.push(data.clone());
            }
        }
    }
    urls
}
