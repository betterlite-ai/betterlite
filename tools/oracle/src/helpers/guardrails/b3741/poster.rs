use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_guardrails::agent_365_tokens::poster::{
    Agent365Poster, Agent365PosterError, PostReply,
};
use betterlite_guardrails::agent_365_tokens::{Agent365Guardrail, Agent365Settings};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap};
use secrecy::SecretString;
use url::form_urlencoded;

use super::{FIXED_NOW_SECONDS, ScriptedReply};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct Recorded {
    url: String,
    content_type: Option<String>,
    form: BTreeMap<String, String>,
}

pub(super) struct ScriptedReplies {
    replies: Vec<PostReply>,
    served: AtomicUsize,
    repeat_last_reply: bool,
    requests: Mutex<Vec<Recorded>>,
}

impl ScriptedReplies {
    pub(super) fn new(
        replies: &[ScriptedReply],
        repeat_last_reply: bool,
    ) -> Result<Self, Box<HelperError>> {
        let mut prepared = Vec::new();
        for reply in replies {
            let status = StatusCode::from_u16(reply.status).or_value_error()?;
            let body = match &reply.raw {
                Some(raw) => raw.as_bytes().to_vec(),
                None => serde_json::to_vec(&reply.body).or_value_error()?,
            };
            prepared.push(PostReply { status, body });
        }
        Ok(Self {
            replies: prepared,
            served: AtomicUsize::new(0),
            repeat_last_reply,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn next_reply(&self) -> Option<PostReply> {
        let index = self.served.fetch_add(1, Ordering::Relaxed);
        match self.replies.get(index) {
            Some(reply) => Some(reply.clone()),
            None if self.repeat_last_reply => self.replies.last().cloned(),
            None => None,
        }
    }

    fn record(&self, url: &str, headers: &HeaderMap, body: &[u8]) {
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let form: BTreeMap<String, String> = form_urlencoded::parse(body)
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: url.to_owned(),
                content_type,
                form,
            });
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl Agent365Poster for ScriptedReplies {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        _timeout: Duration,
    ) -> Result<PostReply, Agent365PosterError> {
        self.record(url, headers, body);
        let reply = self.next_reply();
        reply.ok_or_else(|| Agent365PosterError::NoReply {
            url: url.to_owned(),
        })
    }
}

pub(super) fn guardrail(
    poster: Arc<ScriptedReplies>,
) -> Result<Agent365Guardrail, Box<HelperError>> {
    let settings = Agent365Settings {
        guardrail_name: "agent-365-guard".to_owned(),
        tenant_id: "tenant-abc".to_owned(),
        client_id: "client-xyz".to_owned(),
        client_secret: SecretString::from("secret-123"),
        request_timeout: None,
        unreachable_fallback: UnreachableFallback::FailClosed,
        event_hook: None,
        default_on: false,
    };
    let now = UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS))
        .ok_or("the fixed helper clock is past SystemTime's range")
        .or_config_error()?;
    let clock: Arc<dyn Fn() -> SystemTime + Send + Sync> = Arc::new(move || now);
    Agent365Guardrail::new(settings, poster, true, clock).or_config_error()
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()
}
