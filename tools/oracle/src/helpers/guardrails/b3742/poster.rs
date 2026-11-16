use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::agent_365_tokens::poster::{
    Agent365Poster, Agent365PosterError, PostReply,
};
use betterlite_guardrails::agent_365_tokens::{Agent365Guardrail, Agent365Settings};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap};
use secrecy::SecretString;
use serde_json::Value;
use url::form_urlencoded;

use super::{FIXED_NOW_SECONDS, FORM_CONTENT_TYPE, ScriptedReply};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

// The request the hook sent, in the shape both sides of the oracle read it
#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct Recorded {
    url: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: Value,
}

pub(super) struct ScriptedReplies {
    replies: Vec<Option<PostReply>>,
    served: AtomicUsize,
    repeat_last_reply: bool,
    requests: Mutex<Vec<Recorded>>,
}

impl ScriptedReplies {
    pub(super) fn new(
        replies: &[ScriptedReply],
        repeat_last_reply: bool,
    ) -> Result<Self, Box<HelperError>> {
        let mut prepared = Vec::with_capacity(replies.len());
        for reply in replies {
            prepared.push(prepare(reply)?);
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
            Some(reply) => reply.clone(),
            None if self.repeat_last_reply => self.replies.last().cloned().flatten(),
            None => None,
        }
    }

    fn record(&self, url: &str, headers: &HeaderMap, body: &[u8]) {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let recorded = Recorded {
            url: url.to_owned(),
            body: recorded_body(content_type.as_deref(), body),
            authorization,
            content_type,
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// An unreachable reply is no reply: the hook sees a poster error, as it sees a transport failure
fn prepare(reply: &ScriptedReply) -> Result<Option<PostReply>, Box<HelperError>> {
    let ScriptedReply::Reply { status, body, raw } = reply else {
        return Ok(None);
    };
    let status = StatusCode::from_u16(*status).or_value_error()?;
    let body = match raw {
        Some(raw) => raw.as_bytes().to_vec(),
        None => serde_json::to_vec(body).or_value_error()?,
    };
    Ok(Some(PostReply { status, body }))
}

fn recorded_body(content_type: Option<&str>, body: &[u8]) -> Value {
    if content_type == Some(FORM_CONTENT_TYPE) {
        return form_value(body);
    }
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

fn form_value(body: &[u8]) -> Value {
    let mut fields = serde_json::Map::new();
    for (name, value) in form_urlencoded::parse(body) {
        let _previous = fields.insert(name.into_owned(), Value::String(value.into_owned()));
    }
    Value::Object(fields)
}

pub(super) fn guardrail(
    poster: Arc<ScriptedReplies>,
    fallback: UnreachableFallback,
) -> Result<Agent365Guardrail, Box<HelperError>> {
    let settings = Agent365Settings {
        guardrail_name: "agent-365-guard".to_owned(),
        tenant_id: "tenant-abc".to_owned(),
        client_id: "client-xyz".to_owned(),
        client_secret: SecretString::from("secret-123"),
        request_timeout: None,
        unreachable_fallback: fallback,
        event_hook: Some(LitellmParamsMode::One("pre_mcp_call".to_owned())),
        default_on: true,
    };
    let now = UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS))
        .ok_or("the fixed helper clock is past SystemTime's range")
        .or_config_error()?;
    let clock: Arc<dyn Fn() -> SystemTime + Send + Sync> = Arc::new(move || now);
    Agent365Guardrail::new(settings, poster, true, clock).or_config_error()
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
