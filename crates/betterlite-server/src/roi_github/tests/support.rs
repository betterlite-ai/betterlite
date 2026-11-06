#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::http::header::{ACCEPT, AUTHORIZATION};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_roi::settings::{DEFAULT_PROMPT, ROISettings};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::task::yield_now;

use crate::roi_github::client::GitHub;
use crate::roi_github::transport::Transport;

#[derive(Debug, Clone)]
pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub method: String,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub accept: Option<String>,
    pub authorization: Option<String>,
    pub deadline: Duration,
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
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(&request));
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
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
    Seen {
        method: request.method.to_string(),
        url: request.url.to_string(),
        query: request.query.clone(),
        accept: header(ACCEPT),
        authorization: header(AUTHORIZATION),
        deadline: request.deadline,
    }
}

fn answer_response(reply: Reply) -> UpstreamResponse {
    let mut headers = HeaderMap::new();
    for (name, value) in reply.headers {
        headers.insert(
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
    let settings = ROISettings::default();
    GitHub::new(&settings, script).unwrap()
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

pub(crate) const NEXT_LINK: &str = "<https://api.github.com/user/repos?page=2>; rel=\"next\"";
