#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use secrecy::SecretSlice;
use thiserror::Error;
use uuid::Uuid;

use crate::completion_bridge::EventSource;
use crate::watsonx_credentials::bearer::{
    BearerTokens, TokenClock, TokenHttpResponse, TokenRequest, TokenTransport,
};
use crate::watsonx_credentials::params::WxoLitellmParams;
use crate::watsonx_credentials::token_cache::BoundedTokenCache;
use crate::watsonx_run::handler::RunHandler;
use crate::watsonx_run::transport::{
    MAX_RESPONSE_BYTES, RunFailure, RunLimits, RunRequest, RunResponse, RunTransport, StreamBody,
    StreamResponse,
};
use crate::watsonx_transform::extract::WxoSendParams;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Fail {
    Transport,
    Timeout,
    Read,
    TooLarge,
}

#[derive(Clone, Debug)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) content_type: Option<String>,
    pub(crate) lines: Vec<String>,
    pub(crate) fail: Option<Fail>,
    pub(crate) repeat: bool,
}

impl Reply {
    pub(crate) fn json(body: &str) -> Self {
        Self {
            status: 200,
            body: body.to_owned(),
            content_type: None,
            lines: Vec::new(),
            fail: None,
            repeat: false,
        }
    }

    pub(crate) fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub(crate) fn repeating(mut self) -> Self {
        self.repeat = true;
        self
    }

    pub(crate) fn with_content_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_owned());
        self
    }

    pub(crate) fn failing(fail: Fail) -> Self {
        Self {
            status: 200,
            body: String::new(),
            content_type: None,
            lines: Vec::new(),
            fail: Some(fail),
            repeat: false,
        }
    }

    pub(crate) fn sse(lines: &[&str]) -> Self {
        let mut reply = Self::json("null");
        reply.content_type = Some("text/event-stream; charset=utf-8".to_owned());
        reply.lines = lines.iter().map(|line| (*line).to_owned()).collect();
        reply
    }
}

#[derive(Debug, Error)]
pub(crate) enum ScriptError {
    #[error("the scripted transport has no answer left")]
    Exhausted,
    #[error("the scripted transport failed")]
    Transport,
    #[error("the scripted body cannot be read")]
    Read,
    #[error("the request cannot be recorded")]
    Record(#[source] serde_json::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) method: &'static str,
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: AgentJson,
    pub(crate) timeout: Duration,
}

pub(crate) struct Script {
    replies: Mutex<VecDeque<Reply>>,
    pub(crate) requests: Mutex<Vec<Recorded>>,
    pub(crate) pauses: Mutex<Vec<Duration>>,
    pub(crate) token_calls: Mutex<u32>,
}

impl Script {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
            pauses: Mutex::new(Vec::new()),
            token_calls: Mutex::new(0),
        }
    }

    pub(crate) fn recorded(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn pause_count(&self) -> usize {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn next(&self) -> Result<Reply, ScriptError> {
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(reply) = replies.front() else {
            return Err(ScriptError::Exhausted);
        };
        if reply.repeat {
            return Ok(reply.clone());
        }
        replies.pop_front().ok_or(ScriptError::Exhausted)
    }

    fn reply(
        &self,
        method: &'static str,
        request: &RunRequest<'_>,
    ) -> Result<Reply, RunFailure<ScriptError>> {
        let recorded = request
            .body
            .map_or(Ok(AgentJson::Null), serde_json::to_value)
            .map_err(|source| RunFailure::Transport(ScriptError::Record(source)))?;
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                method,
                url: request.url.to_owned(),
                headers: request.headers.clone(),
                body: recorded,
                timeout: request.limits.timeout,
            });
        let reply = self.next().map_err(RunFailure::Transport)?;
        match reply.fail {
            Some(Fail::Transport) => Err(RunFailure::Transport(ScriptError::Transport)),
            Some(Fail::Timeout) => Err(RunFailure::Timeout),
            _ => Ok(reply),
        }
    }

    fn answer_json(
        &self,
        method: &'static str,
        request: &RunRequest<'_>,
    ) -> Result<RunResponse, RunFailure<ScriptError>> {
        let reply = self.reply(method, request)?;
        Ok(RunResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }

    fn answer_stream(
        &self,
        request: &RunRequest<'_>,
    ) -> Result<StreamResponse<ScriptError>, RunFailure<ScriptError>> {
        let reply = self.reply("POST", request)?;
        let status = reply.status;
        let content_type = reply.content_type.clone().unwrap_or_default();
        let body = match reply.fail {
            Some(Fail::Read) => StreamBody::ReadFailed(ScriptError::Read),
            Some(Fail::TooLarge) => StreamBody::TooLarge {
                max: request.limits.max_response_bytes,
            },
            _ => StreamBody::Chunks(chunks_of(&reply)),
        };
        Ok(StreamResponse {
            status,
            content_type,
            body,
        })
    }
}

fn chunks_of(reply: &Reply) -> Vec<Vec<u8>> {
    let sse = reply
        .content_type
        .as_deref()
        .is_some_and(|value| value.to_lowercase().contains("text/event-stream"));
    if sse {
        return reply
            .lines
            .iter()
            .map(|line| format!("{line}\n").into_bytes())
            .collect();
    }
    vec![reply.body.clone().into_bytes()]
}

impl TokenTransport for &Script {
    type Error = ScriptError;

    fn post(
        &self,
        _request: &TokenRequest,
    ) -> impl Future<Output = Result<TokenHttpResponse, ScriptError>> + Send {
        let mut calls = self
            .token_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *calls = calls.saturating_add(1);
        drop(calls);
        let answer = self.next().and_then(|reply| match reply.fail {
            Some(Fail::Transport) => Err(ScriptError::Transport),
            _ => Ok(TokenHttpResponse {
                status: reply.status,
                body: SecretSlice::from(reply.body.into_bytes()),
            }),
        });
        ready(answer)
    }
}

impl RunTransport for &Script {
    type Error = ScriptError;

    fn post(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, RunFailure<ScriptError>>> + Send {
        ready(self.answer_json("POST", request))
    }

    fn get(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, RunFailure<ScriptError>>> + Send {
        ready(self.answer_json("GET", request))
    }

    fn post_stream(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<StreamResponse<ScriptError>, RunFailure<ScriptError>>> + Send
    {
        ready(self.answer_stream(request))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        ready(())
    }
}

pub(crate) struct Clock {
    pub(crate) monotonic: Duration,
    pub(crate) wall: i64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            monotonic: Duration::ZERO,
            wall: 1_700_000_000,
        }
    }
}

impl TokenClock for Clock {
    fn monotonic(&self) -> Duration {
        self.monotonic
    }

    fn unix_seconds(&self) -> i64 {
        self.wall
    }
}

#[derive(Default)]
pub(crate) struct Source {
    pub(crate) next: u128,
}

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        self.next = self.next.saturating_add(1);
        Uuid::from_u128(self.next)
    }

    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}

pub(crate) fn run_handler(
    script: &Script,
) -> RunHandler<&Script, &Script, Clock, BoundedTokenCache, Source> {
    RunHandler {
        transport: script,
        credentials: BearerTokens {
            transport: script,
            clock: Clock::default(),
            store: BoundedTokenCache::new(NonZeroUsize::new(8).unwrap()),
        },
        source: Source::default(),
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

pub(crate) fn limits(timeout: Duration) -> RunLimits {
    RunLimits {
        timeout,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

pub(crate) fn token_reply() -> Reply {
    Reply::json(r#"{"token": "tok-1", "expiration": 1700001000}"#)
}

pub(crate) const LITELLM_PARAMS: &str = r#"{"cp4d_host": "https://cp4d.example.com///", "instance_id": "inst-1", "wxo_agent_id": "agent-1", "api_key": "key-1", "username": "bob", "auth_mode": "cp4d", "thread_id": "thread-1"}"#;

pub(crate) fn litellm_params() -> WxoLitellmParams {
    serde_json::from_str(LITELLM_PARAMS).unwrap()
}

pub(crate) fn send_params() -> WxoSendParams {
    serde_json::from_str(r#"{"message": {"parts": [{"kind": "text", "text": "hello"}]}}"#).unwrap()
}

pub(crate) fn kwargs() -> BTreeMap<String, AgentJson> {
    BTreeMap::from([(
        "litellm_params".to_owned(),
        serde_json::from_str(LITELLM_PARAMS).unwrap(),
    )])
}
