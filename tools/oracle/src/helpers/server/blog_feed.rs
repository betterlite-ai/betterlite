use crate::helpers::{Helper, HelperError, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

mod date_parse;
mod parser;

pub(crate) struct Op;

const fn default_timeout() -> u64 {
    5
}

const fn default_max_posts() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    LoadLocalBlogPosts,
    FetchRssFeed {
        url: String,
        #[serde(default = "default_timeout")]
        timeout: u64,
        response: MockResponse,
    },
    ParseRssToPosts {
        xml: String,
        #[serde(default = "default_max_posts")]
        max_posts: usize,
    },
    ValidateBlogPosts {
        posts: Value,
    },
    GetBlogPosts {
        url: String,
        now: f64,
        // Stands in for `os.getenv("LITELLM_LOCAL_BLOG_POSTS")`, since the native path never sees case env.
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

impl From<parser::BlogPost> for BlogPost {
    fn from(post: parser::BlogPost) -> Self {
        Self {
            title: post.title,
            description: post.description,
            date: post.date,
            url: post.url,
        }
    }
}

const BLOG_POSTS_TTL_SECONDS: f64 = 3_600.0;

#[derive(Debug, Error)]
enum Failure {
    #[error("HTTP request to {url} failed")]
    Connect { url: String },
    #[error("RSS response returned HTTP status {status}")]
    Status { status: u16 },
    #[error(transparent)]
    Parse(#[from] parser::ParseFailure),
}

impl Failure {
    const fn class(&self) -> &'static str {
        match self {
            Self::Connect { .. } => "ConnectError",
            Self::Status { .. } => "HTTPStatusError",
            Self::Parse(parse_failure) => parse_failure_class(parse_failure),
        }
    }
}

const fn parse_failure_class(error: &parser::ParseFailure) -> &'static str {
    match error {
        parser::ParseFailure::MissingChannel => "ValueError",
        parser::ParseFailure::Xml(_)
        | parser::ParseFailure::Decode(_)
        | parser::ParseFailure::Escape(_)
        | parser::ParseFailure::UnclosedElement => "ParseError",
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct Warning {
    subject: String,
    message: String,
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
            } => json!(fetch_rss_feed(url, *timeout, response)?),
            Input::ParseRssToPosts { xml, max_posts } => {
                let posts: Vec<BlogPost> = parser::parse_rss_to_posts(xml, *max_posts)
                    .map_err(Failure::from)?
                    .into_iter()
                    .map(BlogPost::from)
                    .collect();
                serde_json::to_value(posts).map_err(|error| serialization_error(&error))?
            }
            Input::ValidateBlogPosts { posts } => {
                let (valid, warnings) = validate_blog_posts(posts);
                json!({ "valid": valid, "warnings": warnings })
            }
            Input::GetBlogPosts {
                url,
                now,
                local_only,
                cached_posts,
                last_fetch_time,
                response,
            } => {
                let mut cache = Cache {
                    posts: cached_posts.clone(),
                    last_fetch_time: *last_fetch_time,
                };
                let (posts, warnings) =
                    get_blog_posts(&mut cache, url, *now, *local_only, response);
                serde_json::to_value(json!({ "posts": posts, "warnings": warnings }))
                    .map_err(|error| serialization_error(&error))?
            }
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

// `timeout` is accepted but never mocked: the oracle never actually waits on the clock.
fn fetch_rss_feed(url: &str, _timeout: u64, response: &MockResponse) -> Result<String, Failure> {
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

fn validate_blog_posts(posts: &Value) -> (bool, Vec<Warning>) {
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
struct Cache {
    posts: Option<Vec<BlogPost>>,
    last_fetch_time: f64,
}

impl Cache {
    fn store(&mut self, posts: Vec<BlogPost>, now: f64) {
        self.posts = Some(posts);
        self.last_fetch_time = now;
    }
}

fn get_blog_posts(
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
            class: error.class().to_owned(),
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
