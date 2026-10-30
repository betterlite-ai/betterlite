use crate::helpers::{Helper, HelperError, HelperResult};
use chrono::DateTime;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use thiserror::Error;

pub(crate) struct Op;

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

const fn default_timeout() -> u64 {
    let timeout = 5;
    timeout
}
const fn default_max_posts() -> usize {
    let max_posts = 1;
    max_posts
}

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let value = match input {
            Input::LoadLocalBlogPosts => {
                serde_json::to_value(local_posts()).map_err(serialization_error)?
            }
            Input::FetchRssFeed {
                url,
                timeout,
                response,
            } => json!(fetch_rss_feed(url, *timeout, response)?),
            Input::ParseRssToPosts { xml, max_posts } => {
                serde_json::to_value(parse_rss_to_posts(xml, *max_posts)?)
                    .map_err(serialization_error)?
            }
            Input::ValidateBlogPosts { posts } => json!(validate_blog_posts(posts)),
            Input::GetBlogPosts {
                url,
                now,
                local_only,
                cached_posts,
                last_fetch_time,
                response,
            } => serde_json::to_value(get_blog_posts(
                url,
                *now,
                *local_only,
                cached_posts.as_deref(),
                *last_fetch_time,
                response,
            ))
            .map_err(serialization_error)?,
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

fn parse_rss_to_posts(xml: &str, max_posts: usize) -> Result<Vec<BlogPost>, Failure> {
    let mut reader = Reader::from_str(xml);
    let mut posts = Vec::new();
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut current = String::new();
    let mut in_item = false;
    let mut in_channel = false;
    let mut channel_seen = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
                if name == "channel" {
                    in_channel = true;
                    channel_seen = true;
                } else if name == "item" && in_channel {
                    in_item = true;
                    fields.clear();
                }
                current = name;
            }
            Ok(Event::Text(text)) if in_item => {
                let value = text.decode().map_err(|error| Failure::Parse {
                    message: error.to_string(),
                })?;
                fields.entry(current.clone()).or_default().push_str(&value);
            }
            Ok(Event::CData(text)) if in_item => {
                fields
                    .entry(current.clone())
                    .or_default()
                    .push_str(&String::from_utf8_lossy(text.as_ref()));
            }
            Ok(Event::End(tag)) => {
                let name = String::from_utf8_lossy(tag.name().as_ref());
                if name == "item" {
                    if let (Some(title), Some(url)) = (fields.get("title"), fields.get("link")) {
                        if posts.len() < max_posts {
                            posts.push(BlogPost {
                                title: title.clone(),
                                description: fields.get("description").cloned().unwrap_or_default(),
                                date: parse_date(
                                    fields
                                        .get("pubDate")
                                        .map(String::as_str)
                                        .unwrap_or_default(),
                                ),
                                url: url.clone(),
                            });
                        }
                    }
                    in_item = false;
                    if posts.len() >= max_posts {
                        break;
                    }
                } else if name == "channel" {
                    in_channel = false;
                }
                current.clear();
            }
            Ok(Event::Empty(tag)) if tag.name().as_ref() == b"channel" => {
                channel_seen = true;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(Failure::Parse {
                    message: error.to_string(),
                });
            }
        }
    }
    if channel_seen {
        Ok(posts)
    } else {
        Err(Failure::Parse {
            message: "RSS feed missing <channel> element".to_owned(),
        })
    }
}

fn parse_date(value: &str) -> String {
    let parsed = DateTime::parse_from_rfc2822(value).map_or_else(
        |_| value.to_owned(),
        |date| date.format("%Y-%m-%d").to_string(),
    );
    parsed
}
fn validate_blog_posts(posts: &Value) -> bool {
    let valid = posts.as_array().is_some_and(|items| !items.is_empty());
    valid
}

fn get_blog_posts(
    url: &str,
    now: f64,
    local_only: bool,
    cached_posts: Option<&[BlogPost]>,
    last_fetch_time: f64,
    response: &MockResponse,
) -> Vec<BlogPost> {
    if local_only {
        return local_posts();
    }
    if let Some(posts) = cached_posts.filter(|_| now - last_fetch_time < 3600.0) {
        return posts.to_vec();
    }
    let Ok(xml) = fetch_rss_feed(url, 5, response) else {
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

fn serialization_error(error: serde_json::Error) -> Box<HelperError> {
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
        Box::new(HelperError {
            class: "RuntimeError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}
