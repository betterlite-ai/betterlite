#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_roi::settings::{DEFAULT_PROMPT, ROISettings};
use chrono::NaiveDate;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::task::yield_now;

use crate::roi_github::client::GitHub;
use crate::roi_github::models::{GitHubPullListItem, PullDetail};
use crate::roi_github::transport::Transport;

pub(crate) const NEXT_LINK: &str = "<https://api.github.com/user/repos?page=2>; rel=\"next\"";

#[derive(Debug, Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) authorization: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct Script {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Script {
    #[must_use]
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        self.seen.lock().unwrap().push(recorded(&request));
        let next = self.replies.lock().unwrap().pop_front();
        let Some(answer) = next else {
            return Err(TransportError::BodyTooLarge { limit: 1 });
        };
        Ok(answer_response(answer))
    }
}

fn recorded(request: &OutboundRequest) -> Seen {
    let header = |name: HeaderName| {
        request
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let (body, content_type) = match &request.body {
        RequestBody::Empty => (None, header(CONTENT_TYPE)),
        RequestBody::Json(data) => (
            Some(String::from_utf8(data.clone()).unwrap()),
            header(CONTENT_TYPE).or_else(|| Some("application/json".to_owned())),
        ),
        RequestBody::Bytes { content_type, data } => (
            Some(String::from_utf8(data.clone()).unwrap_or_default()),
            header(CONTENT_TYPE).or_else(|| content_type.to_str().ok().map(str::to_owned)),
        ),
        RequestBody::Multipart { .. } => panic!("the GitHub client sends no multipart bodies"),
    };
    Seen {
        method: request.method.to_string(),
        url: request.url.to_string(),
        query: request.query.clone(),
        authorization: header(AUTHORIZATION),
        content_type,
        body,
    }
}

fn answer_response(reply: Reply) -> UpstreamResponse {
    let mut headers = HeaderMap::new();
    for (name, value) in reply.headers {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(&value).unwrap(),
        );
    }
    UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body,
    }
}

#[must_use]
pub(crate) fn reply(status: u16) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body: Vec::new(),
    }
}

#[must_use]
pub(crate) fn json_reply(status: u16, body: &str) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[must_use]
pub(crate) fn with_link(mut reply: Reply, link: &str) -> Reply {
    reply.headers.push(("link".to_owned(), link.to_owned()));
    reply
}

#[must_use]
pub(crate) fn github(script: Script) -> GitHub<Script> {
    GitHub::new(&ROISettings::default(), script).unwrap()
}

#[must_use]
pub(crate) fn github_with_token(script: Script, token: &str, api_url: &str) -> GitHub<Script> {
    let settings = ROISettings {
        github_api_url: api_url.to_owned(),
        github_token: SecretString::from(token.to_owned()),
        estimator_key: SecretString::default(),
        repos: Vec::new(),
        estimator_model: String::new(),
        estimator_prompt: DEFAULT_PROMPT.to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0,
        identity_map: IndexMap::new(),
    };
    GitHub::new(&settings, script).unwrap()
}

#[must_use]
pub(crate) fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

#[must_use]
pub(crate) fn pull_item(number: i64, merged_at: Option<&str>, updated_at: &str) -> String {
    let merged = merged_at.map_or_else(|| "null".to_owned(), |value| format!("\"{value}\""));
    format!(
        r#"{{"number":{number},"title":"pull {number}","updated_at":"{updated_at}","merged_at":{merged}}}"#
    )
}

#[must_use]
pub(crate) fn json_array(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

#[must_use]
pub(crate) fn pull_list_item(number: i64) -> GitHubPullListItem {
    GitHubPullListItem {
        number,
        html_url: format!("https://github.com/o/r/pull/{number}"),
        merged_at: Some("2026-01-02T00:00:00Z".to_owned()),
        updated_at: "2026-01-02T00:00:00Z".to_owned(),
        title: format!("Pull {number}"),
        body: None,
        head: None,
        user: None,
    }
}

#[must_use]
pub(crate) fn pull_detail_json(
    number: i64,
    login: Option<&str>,
    changed_files: Option<i64>,
    commits: Option<i64>,
) -> String {
    let user = login.map_or_else(
        || "null".to_owned(),
        |name| format!(r#"{{"login":"{name}"}}"#),
    );
    let changed = changed_files.map_or_else(|| "null".to_owned(), |value| format!("{value}"));
    let total = commits.map_or_else(|| "null".to_owned(), |value| format!("{value}"));
    format!(
        r#"{{"number":{number},"title":"Pull {number}","body":null,"html_url":"https://github.com/o/r/pull/{number}","user":{user},"merged_at":"2026-01-02T00:00:00Z","head":{{"sha":"sha-{number}"}},"additions":10,"deletions":4,"changed_files":{changed},"commits":{total}}}"#
    )
}

#[must_use]
pub(crate) fn pull_detail(commits: Option<i64>) -> PullDetail {
    serde_json::from_str(&pull_detail_json(7, Some("octo"), Some(0), commits)).unwrap()
}

#[must_use]
pub(crate) fn evidence_script(detail: &str, files: &str, profile: &str, commits: &str) -> Script {
    Script::new(vec![
        json_reply(200, detail),
        json_reply(200, files),
        json_reply(200, profile),
        json_reply(200, commits),
    ])
}

#[must_use]
pub(crate) fn graphql_page(
    nodes: &str,
    has_next: bool,
    total: i64,
    cursor: Option<&str>,
) -> String {
    let end_cursor = cursor.map_or_else(|| "null".to_owned(), |value| format!("\"{value}\""));
    let page_info = format!(r#"{{"hasNextPage":{has_next},"endCursor":{end_cursor}}}"#);
    let commits = format!(r#"{{"totalCount":{total},"pageInfo":{page_info},"nodes":[{nodes}]}}"#);
    let pull_request = wrap("pullRequest", &wrap("commits", &commits));
    format!(r#"{{"data":{{"repository":{pull_request}}},"errors":[]}}"#)
}

fn wrap(key: &str, value: &str) -> String {
    format!("{{\"{key}\":{value}}}")
}

#[must_use]
pub(crate) fn graphql_error_script(message: &str) -> Script {
    Script::new(vec![json_reply(
        200,
        &format!(r#"{{"data":null,"errors":[{{"message":"{message}"}}]}}"#),
    )])
}

pub(crate) const ENDLESS_GRAPHQL_PAGE: &str = r#"{"data":{"repository":{"pullRequest":{"commits":{"totalCount":1,"pageInfo":{"hasNextPage":true,"endCursor":"cursor"},"nodes":[]}}}},"errors":[]}"#;

#[derive(Debug, Deserialize)]
pub(crate) struct GraphQlRequestBody {
    pub(crate) query: String,
    pub(crate) variables: GraphQlRequestVariables,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GraphQlRequestVariables {
    pub(crate) owner: String,
    pub(crate) name: String,
    pub(crate) number: i64,
    pub(crate) cursor: Option<String>,
}
