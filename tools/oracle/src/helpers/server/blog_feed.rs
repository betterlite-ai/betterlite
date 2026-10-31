use crate::helpers::{Helper, HelperError, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

mod date_parse;
mod logic;
mod parser;

use logic::{Cache, fetch_rss_feed, get_blog_posts, local_posts, validate_blog_posts};

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
    GetBlogPostsSequence {
        calls: Vec<GetBlogPostsCall>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetBlogPostsCall {
    url: String,
    now: f64,
    #[serde(default)]
    local_only: bool,
    #[serde(default)]
    response: MockResponse,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MockResponse {
    status: u16,
    #[serde(default)]
    text: String,
    #[serde(default)]
    error: bool,
}

struct GetRequest<'request> {
    url: &'request str,
    now: f64,
    local_only: bool,
    cached_posts: Option<&'request Vec<BlogPost>>,
    last_fetch_time: f64,
    response: &'request MockResponse,
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
        | parser::ParseFailure::Escape(_)
        | parser::ParseFailure::UnclosedElement
        | parser::ParseFailure::OutsideRoot => "ParseError",
    }
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
            Input::ParseRssToPosts { xml, max_posts } => run_parse(xml, *max_posts)?,
            Input::ValidateBlogPosts { posts } => run_validate(posts),
            Input::GetBlogPosts {
                url,
                now,
                local_only,
                cached_posts,
                last_fetch_time,
                response,
            } => run_get(&GetRequest {
                url,
                now: *now,
                local_only: *local_only,
                cached_posts: cached_posts.as_ref(),
                last_fetch_time: *last_fetch_time,
                response,
            })?,
            Input::GetBlogPostsSequence { calls } => run_sequence(calls),
        };
        Ok(value)
    }
}

fn run_parse(xml: &str, max_posts: usize) -> Result<Value, Box<HelperError>> {
    let posts: Vec<BlogPost> = parser::parse_rss_to_posts(xml, max_posts)
        .map_err(Failure::from)?
        .into_iter()
        .map(BlogPost::from)
        .collect();
    serde_json::to_value(posts).map_err(|error| serialization_error(&error))
}

fn run_validate(posts: &Value) -> Value {
    let (valid, warnings) = validate_blog_posts(posts);
    json!({ "valid": valid, "warnings": warnings })
}

fn run_get(request: &GetRequest<'_>) -> Result<Value, Box<HelperError>> {
    let mut cache = Cache {
        posts: request.cached_posts.cloned(),
        last_fetch_time: request.last_fetch_time,
    };
    let (posts, warnings) = get_blog_posts(
        &mut cache,
        request.url,
        request.now,
        request.local_only,
        request.response,
    );
    serde_json::to_value(json!({ "posts": posts, "warnings": warnings }))
        .map_err(|error| serialization_error(&error))
}

fn run_sequence(calls: &[GetBlogPostsCall]) -> Value {
    let mut cache = Cache {
        posts: None,
        last_fetch_time: 0.0,
    };
    let mut results = Vec::new();
    let mut snapshots = Vec::new();
    let mut fetch_count = 0_u64;
    let mut requests = Vec::new();
    for call in calls {
        let cache_hit = !call.local_only
            && cache.posts.is_some()
            && call.now < cache.last_fetch_time.mul_add(1.0, BLOG_POSTS_TTL_SECONDS);
        let (posts, warnings) = get_blog_posts(
            &mut cache,
            &call.url,
            call.now,
            call.local_only,
            &call.response,
        );
        if !cache_hit && !call.local_only {
            fetch_count = fetch_count.saturating_add(1);
            requests.push(json!({ "url": call.url.clone(), "timeout": default_timeout() }));
        }
        results.push(json!({ "posts": posts, "warnings": warnings }));
        snapshots.push(json!({
            "cached_posts": cache.posts.clone(),
            "last_fetch_time": cache.last_fetch_time,
        }));
    }
    json!({ "results": results, "snapshots": snapshots, "fetch_count": fetch_count, "requests": requests })
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
