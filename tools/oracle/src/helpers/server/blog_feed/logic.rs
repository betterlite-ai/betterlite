use serde_json::Value;

use super::{
    BLOG_POSTS_TTL_SECONDS, BlogPost, Failure, MockResponse, default_max_posts, default_timeout,
    parser,
};

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct Warning {
    pub(super) subject: String,
    pub(super) message: String,
}

pub(super) fn local_posts() -> Vec<BlogPost> {
    let post = BlogPost {
        title: "Realtime WebRTC HTTP Endpoints".to_owned(),
        description: "Use the LiteLLM proxy to route OpenAI-style WebRTC realtime via HTTP: client_secrets and SDP exchange.".to_owned(),
        date: "2026-03-12".to_owned(),
        url: "https://docs.litellm.ai/blog/realtime_webrtc_http_endpoints".to_owned(),
    };
    vec![post]
}

// `timeout` is accepted but never mocked: the oracle never actually waits on the clock.
pub(super) fn fetch_rss_feed(
    url: &str,
    _timeout: u64,
    response: &MockResponse,
) -> Result<String, Failure> {
    if response.error {
        return Err(Failure::Connect {
            url: url.to_owned(),
        });
    }
    if !(200..=299).contains(&response.status) {
        return Err(Failure::Status {
            status: response.status,
        });
    }
    Ok(response.text.clone())
}

fn fetch_and_parse(url: &str, response: &MockResponse) -> Result<Vec<BlogPost>, Failure> {
    let xml = fetch_rss_feed(url, default_timeout(), response)?;
    let posts = parser::parse_rss_to_posts(&xml, default_max_posts())?;
    Ok(posts.into_iter().map(BlogPost::from).collect())
}

pub(super) fn validate_blog_posts(posts: &Value) -> (bool, Vec<Warning>) {
    let valid = posts.as_array().is_some_and(|entries| !entries.is_empty());
    if valid {
        return (valid, Vec::new());
    }
    let warning = Warning {
        subject: "None".to_owned(),
        message: "LiteLLM: Parsed RSS feed has no valid posts. Falling back to local backup."
            .to_owned(),
    };
    (valid, vec![warning])
}

// Mirrors `GetBlogPosts._cached_posts`/`_last_fetch_time`, populated on a successful fetch.
pub(super) struct Cache {
    pub(super) posts: Option<Vec<BlogPost>>,
    pub(super) last_fetch_time: f64,
}

impl Cache {
    pub(super) fn store(&mut self, posts: Vec<BlogPost>, now: f64) {
        self.posts = Some(posts);
        self.last_fetch_time = now;
    }
}

pub(super) fn get_blog_posts(
    cache: &mut Cache,
    url: &str,
    now: f64,
    local_only: bool,
    response: &MockResponse,
) -> (Vec<BlogPost>, Vec<Warning>) {
    if local_only {
        return (local_posts(), Vec::new());
    }
    // Uses `mul_add` for the TTL bound, as `local_ttl_cache.rs` does, to avoid an unsigned subtraction.
    let cache_hit = cache
        .posts
        .clone()
        .filter(|_| now < cache.last_fetch_time.mul_add(1.0, BLOG_POSTS_TTL_SECONDS));
    if let Some(posts) = cache_hit {
        return (posts, Vec::new());
    }
    match fetch_and_parse(url, response) {
        Err(failure) => {
            let warning = Warning {
                subject: url.to_owned(),
                message: format!(
                    "LiteLLM: Failed to fetch blog posts from {url}: {failure}. Falling back to local backup."
                ),
            };
            (local_posts(), vec![warning])
        }
        Ok(posts) if posts.is_empty() => {
            let warning = Warning {
                subject: "None".to_owned(),
                message:
                    "LiteLLM: Parsed RSS feed has no valid posts. Falling back to local backup."
                        .to_owned(),
            };
            (local_posts(), vec![warning])
        }
        Ok(posts) => {
            cache.store(posts.clone(), now);
            (posts, Vec::new())
        }
    }
}
