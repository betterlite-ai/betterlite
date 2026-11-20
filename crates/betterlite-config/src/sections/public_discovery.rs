use indexmap::IndexMap;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const BLOG_POSTS_URL_ENV: &str = "LITELLM_BLOG_POSTS_URL";
const BLOG_POSTS_URL_PATH: &str = "litellm_settings.blog_posts_url";
const DEFAULT_BLOG_POSTS_URL: &str = "https://docs.litellm.ai/blog/rss.xml";
const LOCAL_BLOG_POSTS_ENV: &str = "LITELLM_LOCAL_BLOG_POSTS";
const DOCS_TITLE_ENV: &str = "DOCS_TITLE";
const DEFAULT_DOCS_TITLE: &str = "LiteLLM API";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    blog_posts_url: Option<String>,
    public_mcp_servers: Option<Vec<String>>,
    #[serde(rename = "public_mcp_hub_strict_whitelist")]
    public_mcp_hub_strict: Option<bool>,
    public_model_groups_links: Option<IndexMap<String, UsefulLink>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub blog_posts_url: String,
    pub local_blog_posts: bool,
    pub public_mcp_servers: Option<Vec<String>>,
    pub public_mcp_hub_strict: bool,
    pub public_model_groups_links: IndexMap<String, UsefulLink>,
    pub docs_title: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            blog_posts_url: DEFAULT_BLOG_POSTS_URL.to_owned(),
            local_blog_posts: false,
            public_mcp_servers: None,
            public_mcp_hub_strict: true,
            public_model_groups_links: IndexMap::new(),
            docs_title: DEFAULT_DOCS_TITLE.to_owned(),
        }
    }
}

// LiteLLM types the whole entry as Dict[str, Any], so url and index are kept as written rather than coerced
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct UsefulLinkEntry {
    pub url: Option<JsonValue>,
    pub index: Option<JsonValue>,
    #[serde(flatten)]
    pub other: IndexMap<String, JsonValue>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum UsefulLink {
    Url(String),
    Entry(UsefulLinkEntry),
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let settings = self.litellm_settings;
        let configured_url = settings.blog_posts_url;
        let blog_posts_url = match configured_url {
            Some(url) => resolve_string(url, BLOG_POSTS_URL_PATH, lookup)?,
            None => environment_or(lookup, BLOG_POSTS_URL_ENV, DEFAULT_BLOG_POSTS_URL),
        };
        let local_text = lookup.lookup(LOCAL_BLOG_POSTS_ENV).unwrap_or_default();
        let local_blog_posts =
            betterlite_env::flag(LOCAL_BLOG_POSTS_ENV, &local_text)?.unwrap_or(false);
        Ok(Settings {
            blog_posts_url,
            local_blog_posts,
            public_mcp_servers: settings.public_mcp_servers,
            public_mcp_hub_strict: settings.public_mcp_hub_strict.unwrap_or(true),
            public_model_groups_links: settings.public_model_groups_links.unwrap_or_default(),
            docs_title: environment_or(lookup, DOCS_TITLE_ENV, DEFAULT_DOCS_TITLE),
        })
    }
}

fn environment_or(lookup: &dyn EnvironmentLookup, name: &str, default: &str) -> String {
    lookup
        .lookup(name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}
