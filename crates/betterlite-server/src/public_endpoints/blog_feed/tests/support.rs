#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::public_discovery::Settings;
use indexmap::IndexMap;

use super::super::error::LocalPostsError;
use super::super::local::LocalPosts;
use super::super::testing::RecordingWarnings;
use super::super::{BlogFeed, Clock};
use crate::public_endpoints::blog_feed_parser::BlogPost;
use crate::roi_github::transport::Transport;

pub(super) const FEED_URL: &str = "https://feed.test/rss.xml";
pub(super) const FRESH: &str =
    "<rss><channel><item><title>Fresh</title><link>https://fresh</link></item></channel></rss>";
pub(super) const OTHER: &str =
    "<rss><channel><item><title>Other</title><link>https://other</link></item></channel></rss>";
pub(super) const EMPTY_FEED: &str = "<rss><channel/></rss>";

pub(super) fn settings(url: &str, local_only: bool) -> Settings {
    Settings {
        blog_posts_url: url.to_owned(),
        local_blog_posts: local_only,
        public_mcp_servers: None,
        public_mcp_hub_strict: true,
        public_model_groups_links: IndexMap::new(),
        docs_title: String::new(),
    }
}

pub(super) fn post(title: &str, url: &str) -> BlogPost {
    BlogPost {
        title: title.to_owned(),
        description: String::new(),
        date: String::new(),
        url: url.to_owned(),
    }
}

pub(super) fn backup_post() -> BlogPost {
    post("Backup", "https://backup")
}

pub(super) struct StaticPosts {
    pub(super) posts: Vec<BlogPost>,
}

impl LocalPosts for StaticPosts {
    fn load(&self) -> Result<Vec<BlogPost>, LocalPostsError> {
        Ok(self.posts.clone())
    }
}

pub(super) fn backup_posts() -> StaticPosts {
    StaticPosts {
        posts: vec![backup_post()],
    }
}

pub(super) struct MissingPosts;

impl LocalPosts for MissingPosts {
    fn load(&self) -> Result<Vec<BlogPost>, LocalPostsError> {
        Err(LocalPostsError::Missing)
    }
}

pub(super) fn feed<Client>(
    client: Client,
    url: &str,
    local_only: bool,
    clock: Arc<dyn Clock>,
    source: Arc<dyn LocalPosts>,
) -> (BlogFeed<Client>, Arc<RecordingWarnings>)
where
    Client: Transport,
{
    let warnings = Arc::new(RecordingWarnings::default());
    let settings = settings(url, local_only);
    let sink = Arc::clone(&warnings);
    let feed = BlogFeed::new(client, &settings, clock, source, sink);
    (feed, warnings)
}
