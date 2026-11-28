use std::sync::Arc;

use betterlite_config::sections::public_discovery::Settings;
use betterlite_server::public_endpoints::blog_feed::BlogFeed;
use betterlite_server::public_endpoints::blog_feed::cache::CacheSnapshot;
use betterlite_server::public_endpoints::blog_feed::local::BundledBlogPosts;
use betterlite_server::public_endpoints::blog_feed::testing::{
    Answer, FixedClock, RecordingWarnings, Script, reply,
};
use betterlite_server::public_endpoints::blog_feed::warnings::FeedWarning;
use betterlite_server::public_endpoints::blog_feed_parser::BlogPost;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::MockResponse;

pub(super) fn feed_for(
    script: Script,
    url: &str,
    local_only: bool,
    now: f64,
    warnings: &Arc<RecordingWarnings>,
) -> BlogFeed<Script> {
    let settings = settings_for(url, local_only);
    BlogFeed::new(
        script,
        &settings,
        Arc::new(FixedClock { now }),
        Arc::new(BundledBlogPosts),
        Arc::clone(warnings),
    )
}

pub(super) fn answer_for(response: &MockResponse) -> Answer {
    if response.error {
        return Answer::Closed;
    }
    reply(response.status, &response.text)
}

pub(super) fn settings_for(url: &str, local_only: bool) -> Settings {
    Settings {
        blog_posts_url: url.to_owned(),
        local_blog_posts: local_only,
        public_mcp_servers: None,
        public_mcp_hub_strict: true,
        public_model_groups_links: IndexMap::new(),
        docs_title: String::new(),
    }
}

pub(super) fn warning_value(warning: &FeedWarning) -> Value {
    json!({
        "subject": warning.subject.as_deref().unwrap_or("None"),
        "message": warning.message.as_str(),
    })
}

pub(super) fn call_value(posts: &[BlogPost], warnings: &RecordingWarnings) -> Value {
    let entries = warnings.warnings();
    json!({
        "posts": posts,
        "warnings": entries.iter().map(warning_value).collect::<Vec<Value>>(),
    })
}

pub(super) fn snapshot_value(snapshot: &CacheSnapshot) -> Value {
    json!({
        "cached_posts": snapshot.cached_posts,
        "last_fetch_time": snapshot.last_fetch_time,
    })
}
