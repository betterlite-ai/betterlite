use std::sync::Arc;
use std::time::Duration;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::public_endpoints::blog_feed::BlogFeed;
use betterlite_server::public_endpoints::blog_feed::local::BundledBlogPosts;
use betterlite_server::public_endpoints::blog_feed::testing::{
    FixedClock, RecordingWarnings, Script,
};
use betterlite_server::public_endpoints::blog_feed::warnings::{FeedWarning, validate_blog_posts};
use betterlite_server::public_endpoints::blog_feed_parser::{BlogPost, parse_rss_to_posts};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod logic;

use logic::{answer_for, call_value, feed_for, settings_for, snapshot_value, warning_value};

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

struct GetCall<'call> {
    url: &'call str,
    now: f64,
    local_only: bool,
    cached: Option<&'call Vec<BlogPost>>,
    last_fetch_time: f64,
    response: &'call MockResponse,
}

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let value = match input {
            Input::LoadLocalBlogPosts => local_answer()?,
            Input::FetchRssFeed {
                url,
                timeout,
                response,
            } => fetch_answer(url, *timeout, response)?,
            Input::ParseRssToPosts { xml, max_posts } => serde_json::to_value(
                parse_rss_to_posts(xml, *max_posts, usize::MAX).or_value_error()?,
            )?,
            Input::ValidateBlogPosts { posts } => validate_answer(posts),
            Input::GetBlogPosts {
                url,
                now,
                local_only,
                cached_posts,
                last_fetch_time,
                response,
            } => get_answer(&GetCall {
                url,
                now: *now,
                local_only: *local_only,
                cached: cached_posts.as_ref(),
                last_fetch_time: *last_fetch_time,
                response,
            })?,
            Input::GetBlogPostsSequence { calls } => sequence_answer(calls)?,
        };
        Ok(value)
    }
}

fn local_answer() -> HelperResult {
    let settings = settings_for("", true);
    let script = Script::new(Vec::new());
    let feed = BlogFeed::new(
        script,
        &settings,
        Arc::new(FixedClock { now: 0.0 }),
        Arc::new(BundledBlogPosts),
        Arc::new(RecordingWarnings::default()),
    );
    let runtime = Builder::new_current_thread().build()?;
    let posts = runtime.block_on(feed.get_blog_posts()).or_value_error()?;
    Ok(serde_json::to_value(posts)?)
}

fn fetch_answer(url: &str, timeout: u64, response: &MockResponse) -> HelperResult {
    let script = Script::new(vec![answer_for(response)]);
    let warnings = Arc::new(RecordingWarnings::default());
    let feed = feed_for(script.clone(), url, false, 0.0, &warnings);
    let runtime = Builder::new_current_thread().build()?;
    let text = runtime
        .block_on(feed.fetch_rss_feed(url, Duration::from_secs(timeout)))
        .or_value_error()?;
    let request = script
        .seen()
        .into_iter()
        .next()
        .map(|seen| json!({ "url": seen.url, "timeout": seen.timeout_seconds }));
    Ok(json!({ "text": text, "request": request }))
}

fn get_answer(call: &GetCall<'_>) -> HelperResult {
    let script = Script::new(vec![answer_for(call.response)]);
    let warnings = Arc::new(RecordingWarnings::default());
    let feed = feed_for(script, call.url, call.local_only, call.now, &warnings);
    feed.seed_cache(call.cached.cloned(), call.last_fetch_time);
    let runtime = Builder::new_current_thread().build()?;
    let posts = runtime.block_on(feed.get_blog_posts()).or_value_error()?;
    Ok(call_value(&posts, &warnings))
}

fn sequence_answer(calls: &[GetBlogPostsCall]) -> HelperResult {
    let runtime = Builder::new_current_thread().build()?;
    let mut results = Vec::new();
    let mut snapshots = Vec::new();
    let mut requests = Vec::new();
    let mut cached: Option<Vec<BlogPost>> = None;
    let mut last_fetch_time = 0.0;
    for call in calls {
        let script = Script::new(vec![answer_for(&call.response)]);
        let warnings = Arc::new(RecordingWarnings::default());
        let feed = feed_for(
            script.clone(),
            &call.url,
            call.local_only,
            call.now,
            &warnings,
        );
        feed.seed_cache(cached.take(), last_fetch_time);
        let posts = runtime.block_on(feed.get_blog_posts()).or_value_error()?;
        for seen in script.seen() {
            requests.push(json!({ "url": seen.url, "timeout": seen.timeout_seconds }));
        }
        results.push(call_value(&posts, &warnings));
        let snapshot = feed.cache_snapshot();
        snapshots.push(snapshot_value(&snapshot));
        cached = snapshot.cached_posts;
        last_fetch_time = snapshot.last_fetch_time;
    }
    Ok(json!({
        "results": results,
        "snapshots": snapshots,
        "fetch_count": requests.len(),
        "requests": requests,
    }))
}

fn validate_answer(posts: &Value) -> Value {
    let Some(entries) = posts.as_array() else {
        return json!({
            "valid": false,
            "warnings": [warning_value(&FeedWarning::empty_posts())],
        });
    };
    match validate_blog_posts(entries) {
        Ok(()) => json!({ "valid": true, "warnings": [] }),
        Err(warning) => json!({
            "valid": false,
            "warnings": [warning_value(&warning)],
        }),
    }
}

#[path = "blog_feed/tests.rs"]
mod tests;
