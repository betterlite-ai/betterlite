#![cfg(test)]

mod support;
mod transport;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use self::support::{
    EMPTY_FEED, FEED_URL, FRESH, MissingPosts, OTHER, backup_post, backup_posts, feed, post,
};
use super::Clock as _;
use super::error::{BlogFeedError, LocalPostsError};
use super::local::BundledBlogPosts;
use super::testing::{Answer, FixedClock, Script, StepClock, reply};
use super::warnings::{FeedWarning, WarnLog, WarningSink};
use super::{BlogPostsResponse, MAX_FEED_BYTES, SystemClock};

const SIXTEEN_MEBIBYTES: usize = 16 * 1024 * 1024;

fn title_nested_in(inline_elements: usize) -> String {
    let mut xml = String::from("<rss><channel><item><title>Deep");
    let opened = "<b>".repeat(inline_elements);
    let closed = "</b>".repeat(inline_elements);
    xml.push_str(&opened);
    xml.push_str(&closed);
    xml.push_str("</title><link>https://deep</link></item></channel></rss>");
    xml
}

fn capped_feed(size: usize) -> String {
    let head = "<rss><channel><item><title>";
    let tail = "</title><link>https://capped</link></item></channel></rss>";
    let overhead = head.len().checked_add(tail.len()).unwrap();
    let padding = "a".repeat(size.checked_sub(overhead).unwrap());
    format!("{head}{padding}{tail}")
}

#[tokio::test]
async fn local_mode_skips_fetch() {
    let script = Script::new(vec![reply(200, FRESH)]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(
        script.clone(),
        FEED_URL,
        true,
        clock,
        Arc::new(BundledBlogPosts),
    );
    feed.seed_cache(Some(vec![post("Cached", "https://cached")]), 100.0);
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "Realtime WebRTC HTTP Endpoints");
    assert_eq!(
        posts[0].url,
        "https://docs.litellm.ai/blog/realtime_webrtc_http_endpoints"
    );
    assert!(
        posts[0]
            .description
            .starts_with("Use the LiteLLM proxy to route OpenAI-style WebRTC")
    );
    assert!(script.seen().is_empty());
    assert!(warnings.warnings().is_empty());
    let snapshot = feed.cache_snapshot();
    assert_eq!(
        snapshot.cached_posts,
        Some(vec![post("Cached", "https://cached")])
    );
    assert_eq!(snapshot.last_fetch_time, 100.0);
}

#[tokio::test]
async fn success_is_cached() {
    let script = Script::new(vec![reply(200, FRESH), reply(200, OTHER)]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(
        script.clone(),
        FEED_URL,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    let first = feed.get_blog_posts().await.unwrap();
    let second = feed.get_blog_posts().await.unwrap();
    assert_eq!(first, vec![post("Fresh", "https://fresh")]);
    assert_eq!(second, first);
    assert_eq!(script.seen().len(), 1);
    assert_eq!(script.seen()[0].url, FEED_URL);
    assert_eq!(script.seen()[0].timeout_seconds, 5);
    assert!(warnings.warnings().is_empty());
    let snapshot = feed.cache_snapshot();
    assert_eq!(snapshot.cached_posts, Some(first));
    assert_eq!(snapshot.last_fetch_time, 100.0);
}

#[tokio::test]
async fn expires_at_one_hour() {
    let clock = Arc::new(StepClock::new([3699.0, 3700.0]));
    let script = Script::new(vec![reply(200, OTHER)]);
    let (feed, _) = feed(
        script.clone(),
        FEED_URL,
        false,
        Arc::clone(&clock),
        Arc::new(backup_posts()),
    );
    feed.seed_cache(Some(vec![post("Old", "https://old")]), 100.0);
    let cached = feed.get_blog_posts().await.unwrap();
    assert_eq!(cached, vec![post("Old", "https://old")]);
    assert!(script.seen().is_empty());
    let fetched = feed.get_blog_posts().await.unwrap();
    assert_eq!(fetched, vec![post("Other", "https://other")]);
    assert_eq!(script.seen().len(), 1);
    assert_eq!(clock.reads.load(Ordering::SeqCst), 2);
    assert_eq!(feed.cache_snapshot().last_fetch_time, 3700.0);
}

#[tokio::test]
async fn timestamp_precedes_fetch() {
    let clock = Arc::new(StepClock::new([100.0, 500.0]));
    let script = Script::new(vec![reply(200, FRESH)]);
    let (feed, _) = feed(
        script,
        FEED_URL,
        false,
        Arc::clone(&clock),
        Arc::new(backup_posts()),
    );
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![post("Fresh", "https://fresh")]);
    assert_eq!(clock.reads.load(Ordering::SeqCst), 1);
    assert_eq!(feed.cache_snapshot().last_fetch_time, 100.0);
}

#[tokio::test]
async fn failed_refresh_keeps_cache() {
    let clock = Arc::new(StepClock::new([4000.0, 4001.0]));
    let script = Script::new(vec![reply(503, ""), reply(200, OTHER)]);
    let (feed, warnings) = feed(
        script.clone(),
        FEED_URL,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    feed.seed_cache(Some(vec![post("Old", "https://old")]), 100.0);
    let fallback = feed.get_blog_posts().await.unwrap();
    assert_eq!(fallback, vec![backup_post()]);
    let snapshot = feed.cache_snapshot();
    assert_eq!(
        snapshot.cached_posts,
        Some(vec![post("Old", "https://old")])
    );
    assert_eq!(snapshot.last_fetch_time, 100.0);
    assert_eq!(warnings.warnings().len(), 1);
    assert_eq!(warnings.warnings()[0].subject.as_deref(), Some(FEED_URL));
    let refreshed = feed.get_blog_posts().await.unwrap();
    assert_eq!(refreshed, vec![post("Other", "https://other")]);
    assert_eq!(script.seen().len(), 2);
    assert_eq!(feed.cache_snapshot().last_fetch_time, 4001.0);
}

#[tokio::test]
async fn fallback_is_not_cached() {
    let script = Script::new(vec![reply(200, "<rss>")]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(script, FEED_URL, false, clock, Arc::new(backup_posts()));
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    let snapshot = feed.cache_snapshot();
    assert_eq!(snapshot.cached_posts, None);
    assert_eq!(snapshot.last_fetch_time, 0.0);
    let warnings = warnings.warnings();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].subject.as_deref(), Some(FEED_URL));
    assert!(warnings[0].message.contains("never closed"));
    assert!(warnings[0].message.starts_with(&format!(
        "LiteLLM: Failed to fetch blog posts from {FEED_URL}: "
    )));
    assert!(
        warnings[0]
            .message
            .ends_with(". Falling back to local backup.")
    );
}

#[tokio::test]
async fn non_utf8_body_falls_back() {
    let script = Script::new(vec![Answer::Reply {
        status: 200,
        body: vec![0xff, 0xfe],
    }]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(script, FEED_URL, false, clock, Arc::new(backup_posts()));
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    assert_eq!(feed.cache_snapshot().cached_posts, None);
    let warnings = warnings.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("not valid UTF-8"));
}

#[test]
fn a_response_without_posts_is_empty() {
    let response: BlogPostsResponse = serde_json::from_str("{}").unwrap();
    assert!(response.posts.is_empty());
}

#[tokio::test]
async fn empty_feed_falls_back() {
    let script = Script::new(vec![reply(200, EMPTY_FEED)]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(script, FEED_URL, false, clock, Arc::new(backup_posts()));
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    assert_eq!(warnings.warnings(), vec![FeedWarning::empty_posts()]);
    assert_eq!(feed.cache_snapshot().cached_posts, None);
}

#[tokio::test]
async fn deep_nesting_falls_back() {
    let at_limit = Script::new(vec![reply(200, &title_nested_in(60))]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(at_limit, FEED_URL, false, clock, Arc::new(backup_posts()));
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![post("Deep", "https://deep")]);
    assert!(warnings.warnings().is_empty());
    let past_limit = Script::new(vec![reply(200, &title_nested_in(61))]);
    let later = Arc::new(FixedClock { now: 200.0 });
    let (feed, warnings) = feed(past_limit, FEED_URL, false, later, Arc::new(backup_posts()));
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    assert_eq!(feed.cache_snapshot().cached_posts, None);
    let warnings = warnings.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("nests deeper than 64"));
}

#[tokio::test]
async fn backwards_clock_keeps_cache() {
    let clock = Arc::new(StepClock::new([500.0]));
    let script = Script::new(Vec::new());
    let (feed, _) = feed(
        script.clone(),
        FEED_URL,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    feed.seed_cache(Some(vec![post("Old", "https://old")]), 1000.0);
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![post("Old", "https://old")]);
    assert!(script.seen().is_empty());
    assert_eq!(feed.cache_snapshot().last_fetch_time, 1000.0);
}

#[tokio::test]
async fn backup_failure_propagates() {
    let script = Script::new(vec![reply(503, ""), reply(200, EMPTY_FEED)]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, _) = feed(script, FEED_URL, false, clock, Arc::new(MissingPosts));
    let failed = feed.get_blog_posts().await.unwrap_err();
    assert!(matches!(
        failed,
        BlogFeedError::Local(LocalPostsError::Missing)
    ));
    let empty = feed.get_blog_posts().await.unwrap_err();
    assert!(matches!(
        empty,
        BlogFeedError::Local(LocalPostsError::Missing)
    ));
}

#[tokio::test]
async fn independent_apps() {
    let script_a = Script::new(vec![reply(200, FRESH)]);
    let script_b = Script::new(vec![reply(200, OTHER)]);
    let (feed_a, _) = feed(
        script_a.clone(),
        FEED_URL,
        false,
        Arc::new(FixedClock { now: 100.0 }),
        Arc::new(backup_posts()),
    );
    let (feed_b, _) = feed(
        script_b.clone(),
        FEED_URL,
        false,
        Arc::new(FixedClock { now: 200.0 }),
        Arc::new(backup_posts()),
    );
    let first_a = feed_a.get_blog_posts().await.unwrap();
    let first_b = feed_b.get_blog_posts().await.unwrap();
    let second_a = feed_a.get_blog_posts().await.unwrap();
    let second_b = feed_b.get_blog_posts().await.unwrap();
    assert_eq!(first_a, vec![post("Fresh", "https://fresh")]);
    assert_eq!(first_b, vec![post("Other", "https://other")]);
    assert_eq!(second_a, first_a);
    assert_eq!(second_b, first_b);
    assert_eq!(script_a.seen().len(), 1);
    assert_eq!(script_b.seen().len(), 1);
}

#[tokio::test]
async fn accepts_a_body_at_the_size_cap() {
    assert_eq!(MAX_FEED_BYTES, SIXTEEN_MEBIBYTES);
    let body = capped_feed(SIXTEEN_MEBIBYTES);
    let script = Script::new(vec![reply(200, &body)]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(script, FEED_URL, false, clock, Arc::new(backup_posts()));
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, "https://capped");
    assert!(warnings.warnings().is_empty());
    assert!(feed.cache_snapshot().cached_posts.is_some());
}

#[tokio::test]
async fn rejects_a_body_over_the_size_cap() {
    let oversized = MAX_FEED_BYTES.checked_add(1).unwrap();
    let body = capped_feed(oversized);
    let script = Script::new(vec![reply(200, &body)]);
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(script, FEED_URL, false, clock, Arc::new(backup_posts()));
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    assert_eq!(feed.cache_snapshot().cached_posts, None);
    let warnings = warnings.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .message
            .contains(&format!("larger than {MAX_FEED_BYTES} bytes"))
    );
}

#[test]
fn system_clock_reads_epoch_seconds() {
    let now = Duration::try_from_secs_f64(SystemClock.now_seconds()).unwrap();
    assert!(now > Duration::from_secs(1_600_000_000));
}

#[test]
fn warn_log_reports_the_warning_and_its_subject() {
    let warning = FeedWarning {
        subject: Some("https://feed.test/rss.xml".to_owned()),
        message: "the feed could not be fetched".to_owned(),
    };
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    with_default(subscriber, || WarningSink::warn(&WarnLog, &warning));
    let text = fs::read_to_string(log.path()).unwrap();
    assert!(text.contains("WARN"));
    assert!(text.contains("the feed could not be fetched"));
    assert!(text.contains("https://feed.test/rss.xml"));
}
