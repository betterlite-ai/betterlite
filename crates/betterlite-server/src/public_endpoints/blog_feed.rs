pub mod cache;
pub mod error;
pub mod local;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod warnings;

mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::public_discovery::Settings;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::{HeaderMap, Method};
use url::Url;

use self::cache::Cache;
#[cfg(any(test, feature = "testing"))]
use self::cache::CacheSnapshot;
use self::error::{BlogFeedError, FetchError};
use self::local::LocalPosts;
use self::warnings::{FeedWarning, WarningSink, validate_blog_posts};
use crate::public_endpoints::blog_feed_parser::{BlogPost, parse_rss_to_posts};
use crate::roi_github::transport::Transport;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_FEED_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_POSTS: usize = 1;
pub const MAX_DEPTH: usize = 64;

pub trait Clock: Send + Sync {
    fn now_seconds(&self) -> f64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_seconds(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlogPostsResponse {
    #[serde(default)]
    pub posts: Vec<BlogPost>,
}

pub struct BlogFeed<Client> {
    client: Client,
    source: Arc<dyn LocalPosts>,
    warnings: Arc<dyn WarningSink>,
    clock: Arc<dyn Clock>,
    url: String,
    local_only: bool,
    cache: Mutex<Cache>,
}

impl<Client> BlogFeed<Client>
where
    Client: Transport,
{
    #[must_use]
    pub fn new(
        client: Client,
        settings: &Settings,
        clock: Arc<dyn Clock>,
        source: Arc<dyn LocalPosts>,
        warnings: Arc<dyn WarningSink>,
    ) -> Self {
        Self {
            client,
            url: settings.blog_posts_url.clone(),
            local_only: settings.local_blog_posts,
            clock,
            source,
            warnings,
            cache: Mutex::new(Cache::default()),
        }
    }

    pub async fn get_blog_posts(&self) -> Result<Vec<BlogPost>, BlogFeedError> {
        if self.local_only {
            return Ok(self.source.load()?);
        }
        let now = self.clock.now_seconds();
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fresh(now);
        if let Some(posts) = cached {
            return Ok(posts);
        }
        let parsed = match self.fetch_rss_feed(&self.url, FETCH_TIMEOUT).await {
            Ok(text) => {
                parse_rss_to_posts(&text, MAX_POSTS, MAX_DEPTH).map_err(BlogFeedError::Parse)
            }
            Err(failure) => Err(BlogFeedError::Fetch(failure)),
        };
        let posts = match parsed {
            Ok(posts) => posts,
            Err(failure) => return self.fall_back(&failure),
        };
        if let Err(warning) = validate_blog_posts(&posts) {
            self.warnings.warn(&warning);
            return Ok(self.source.load()?);
        }
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store(posts.clone(), now);
        Ok(posts)
    }

    pub async fn fetch_rss_feed(&self, url: &str, timeout: Duration) -> Result<String, FetchError> {
        let target = Url::parse(url).map_err(|source| FetchError::Url {
            url: url.to_owned(),
            source,
        })?;
        let request = OutboundRequest {
            method: Method::GET,
            url: target,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: timeout,
        };
        let response = self
            .client
            .send(request)
            .await
            .map_err(FetchError::Request)?;
        if !(200..300).contains(&response.status) {
            return Err(FetchError::Status {
                status: response.status,
            });
        }
        if response.body.len() > MAX_FEED_BYTES {
            return Err(FetchError::TooLarge {
                limit: MAX_FEED_BYTES,
            });
        }
        String::from_utf8(response.body).map_err(FetchError::Utf8)
    }

    fn fall_back(&self, failure: &BlogFeedError) -> Result<Vec<BlogPost>, BlogFeedError> {
        let url = self.url.as_str();
        self.warnings.warn(&FeedWarning {
            subject: Some(url.to_owned()),
            message: format!(
                "LiteLLM: Failed to fetch blog posts from {url}: {failure}. Falling back to local backup."
            ),
        });
        Ok(self.source.load()?)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn seed_cache(&self, posts: Option<Vec<BlogPost>>, last_fetch_time: f64) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.posts = posts;
        cache.last_fetch_time = last_fetch_time;
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn cache_snapshot(&self) -> CacheSnapshot {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        CacheSnapshot {
            cached_posts: cache.posts.clone(),
            last_fetch_time: cache.last_fetch_time,
        }
    }
}
