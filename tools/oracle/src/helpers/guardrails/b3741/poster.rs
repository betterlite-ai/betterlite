use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_guardrails::agent_365_tokens::poster::{
    Agent365Poster, Agent365PosterError, PostReply,
};
use betterlite_guardrails::agent_365_tokens::{Agent365Guardrail, Agent365Settings};
use betterlite_http::proxy::limits::ReadError;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap};
use secrecy::SecretString;
use url::form_urlencoded;

use super::{FIXED_NOW, ScriptedReply};
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Clone, serde::Serialize)]
struct Recorded {
    url: String,
    content_type: Option<String>,
    form: BTreeMap<String, String>,
}

pub(super) struct ScriptedReplies {
    replies: Vec<ScriptedReply>,
    served: Mutex<usize>,
    repeat_last_reply: bool,
    requests: Mutex<Vec<Recorded>>,
}

impl ScriptedReplies {
    pub(super) fn new(replies: &[ScriptedReply], repeat_last_reply: bool) -> Self {
        Self {
            replies: replies.to_vec(),
            served: Mutex::new(0),
            repeat_last_reply,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn next_reply(&self) -> Option<ScriptedReply> {
        let mut served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(reply) = self.replies.get(*served) {
            *served = served.saturating_add(1);
            return Some(reply.clone());
        }
        if self.repeat_last_reply {
            return self.replies.last().cloned();
        }
        None
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

// A scripted reply no case produces: the poster's error surfaces as a token error instead.
fn script_error(url: &str) -> Agent365PosterError {
    Agent365PosterError::Body {
        url: url.to_owned(),
        source: ReadError::TooLarge { limit: 0 },
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
        let reply = self.next_reply().ok_or_else(|| script_error(url))?;
        let status = StatusCode::from_u16(reply.status).map_err(|_invalid| script_error(url))?;
        let body = match reply.raw {
            Some(raw) => raw.into_bytes(),
            None => serde_json::to_vec(&reply.body).map_err(|_encode| script_error(url))?,
        };
        Ok(PostReply { status, body })
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
    let mut guardrail = Agent365Guardrail::new(settings, poster, true).or_config_error()?;
    guardrail.clock = Arc::new(|| FIXED_NOW);
    Ok(guardrail)
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()
}
