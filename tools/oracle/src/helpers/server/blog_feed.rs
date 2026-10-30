use crate::helpers::{Helper, HelperError, HelperResult};
use parser::parse_rss_to_posts;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use thiserror::Error;

mod parser;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    LoadLocalBlogPosts,
    FetchRssFeed {
        url: String,
        #[serde(default)]
        timeout: Option<u64>,
        response: MockResponse,
    },
    ParseRssToPosts {
        xml: String,
        #[serde(default)]
        max_posts: Option<usize>,
    },
    ValidateBlogPosts {
        posts: Value,
    },
    GetBlogPosts {
        url: String,
        now: f64,
        #[serde(default)]
        local_only: bool,
        #[serde(default)]
        cached_posts: Option<Vec<BlogPost>>,
        #[serde(default)]
        last_fetch_time: f64,
        response: MockResponse,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MockResponse {
    status: u16,
    #[serde(default)]
    text: String,
    #[serde(default)]
    error: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BlogPost {
    title: String,
    description: String,
    date: String,
    url: String,
}

#[derive(Debug, Error)]
enum Failure {
    #[error("HTTP request to {url} failed after {timeout} seconds")]
    Fetch { url: String, timeout: u64 },
    #[error("RSS response returned HTTP status {status}")]
    Status { status: u16 },
    #[error("RSS parse failed: {message}")]
    Parse { message: String },
}

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let value = match input {
            Input::LoadLocalBlogPosts => {
                serde_json::to_value(local_posts()).map_err(|error| serialization_error(&error))?
            }
            Input::FetchRssFeed {
                url,
                timeout,
                response,
            } => json!(fetch_rss_feed(url, timeout.unwrap_or(5), response)?),
            Input::ParseRssToPosts { xml, max_posts } => {
                serde_json::to_value(parse_rss_to_posts(xml, max_posts.unwrap_or(1))?)
                    .map_err(|error| serialization_error(&error))?
            }
            Input::ValidateBlogPosts { posts } => json!(validate_blog_posts(posts)),
            Input::GetBlogPosts {
                url,
                now,
                local_only,
                cached_posts,
                last_fetch_time,
                response,
            } => serde_json::to_value(get_blog_posts(&BlogPostsRequest {
                url,
                now: *now,
                local_only: *local_only,
                cached_posts: cached_posts.as_deref(),
                last_fetch_time: *last_fetch_time,
                response,
            }))
            .map_err(|error| serialization_error(&error))?,
        };
        Ok(value)
    }
}

fn local_posts() -> Vec<BlogPost> {
    let post = BlogPost {
        title: "Realtime WebRTC HTTP Endpoints".to_owned(),
        description: "Use the LiteLLM proxy to route OpenAI-style WebRTC realtime via HTTP: client_secrets and SDP exchange.".to_owned(),
        date: "2026-03-12".to_owned(),
        url: "https://docs.litellm.ai/blog/realtime_webrtc_http_endpoints".to_owned(),
    };
    vec![post]
}

fn fetch_rss_feed(url: &str, timeout: u64, response: &MockResponse) -> Result<String, Failure> {
    if response.error {
        return Err(Failure::Fetch {
            url: url.to_owned(),
            timeout,
        });
    }
    if !(200..=299).contains(&response.status) {
        return Err(Failure::Status {
            status: response.status,
        });
    }
    Ok(response.text.clone())
}

fn validate_blog_posts(posts: &Value) -> bool {
    let items = posts.as_array();
    if items.is_none() {
        return false;
    }
    items.is_some_and(|entries| !entries.is_empty())
}

struct BlogPostsRequest<'request> {
    url: &'request str,
    now: f64,
    local_only: bool,
    cached_posts: Option<&'request [BlogPost]>,
    last_fetch_time: f64,
    response: &'request MockResponse,
}

fn get_blog_posts(request: &BlogPostsRequest<'_>) -> Vec<BlogPost> {
    if request.local_only {
        return local_posts();
    }
    if let Some(posts) = request
        .cached_posts
        .filter(|_| within_ttl(request.now, request.last_fetch_time))
    {
        return posts.to_vec();
    }
    let Ok(xml) = fetch_rss_feed(request.url, 5, request.response) else {
        return local_posts();
    };
    let Ok(posts) = parse_rss_to_posts(&xml, 1) else {
        return local_posts();
    };
    if posts.is_empty() {
        local_posts()
    } else {
        posts
    }
}

fn within_ttl(now_seconds: f64, last_fetch_time_seconds: f64) -> bool {
    let Ok(now) = Duration::try_from_secs_f64(now_seconds) else {
        return false;
    };
    let Ok(last_fetch_time) = Duration::try_from_secs_f64(last_fetch_time_seconds) else {
        return false;
    };
    now.checked_sub(last_fetch_time)
        .is_some_and(|elapsed| elapsed < Duration::from_secs(3_600))
}

fn serialization_error(error: &serde_json::Error) -> Box<HelperError> {
    let helper_error = HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    };
    Box::new(helper_error)
}
impl From<Failure> for Box<HelperError> {
    fn from(error: Failure) -> Self {
        Self::new(HelperError {
            class: "RuntimeError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}

#[path = "blog_feed/tests.rs"]
mod tests;
