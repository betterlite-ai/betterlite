use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_a2a::completion_bridge::EventSource;
use betterlite_a2a::watsonx_credentials::bearer::{
    TokenClock, TokenHttpResponse, TokenRequest, TokenTransport,
};
use betterlite_a2a::watsonx_run::transport::{
    RunRequest, RunResponse, RunTransport, StreamBody, StreamResponse,
};
use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use secrecy::SecretSlice;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use super::{Fail, Step};

#[derive(Debug, Error)]
pub(super) enum ScriptError {
    #[error("the scripted transport has no answer left")]
    Exhausted,
    #[error("the scripted transport failed")]
    Transport,
    #[error("the scripted body cannot be read")]
    Read,
    #[error("the answer cannot be recorded")]
    Record(#[source] serde_json::Error),
}

#[derive(Serialize)]
pub(super) struct Recorded {
    pub(super) method: &'static str,
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: AgentJson,
    pub(super) timeout: f64,
}

pub(super) struct Script {
    steps: Mutex<VecDeque<Step>>,
    pub(super) requests: Mutex<Vec<Recorded>>,
    pub(super) pauses: Mutex<Vec<u64>>,
}

impl Script {
    pub(super) fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(VecDeque::from(steps)),
            requests: Mutex::new(Vec::new()),
            pauses: Mutex::new(Vec::new()),
        }
    }

    fn next(&self) -> Result<Step, ScriptError> {
        let mut steps = self.steps.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(step) = steps.front() else {
            return Err(ScriptError::Exhausted);
        };
        if step.repeat {
            return Ok(step.clone());
        }
        steps.pop_front().ok_or(ScriptError::Exhausted)
    }

    fn record(&self, method: &'static str, request: &RunRequest<'_>) -> Result<(), ScriptError> {
        let body = match request.body {
            Some(body) => serde_json::to_value(body).map_err(ScriptError::Record)?,
            None => AgentJson::Null,
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                method,
                url: request.url.to_owned(),
                headers: request.headers.clone(),
                body,
                timeout: request.timeout.as_secs_f64(),
            });
        Ok(())
    }

    fn answer_json(
        &self,
        method: &'static str,
        request: &RunRequest<'_>,
    ) -> Result<RunResponse, ScriptError> {
        self.record(method, request)?;
        let step = self.next()?;
        if matches!(step.fail, Some(Fail::Transport)) {
            return Err(ScriptError::Transport);
        }
        let body = serde_json::to_vec(&step.body).map_err(ScriptError::Record)?;
        Ok(RunResponse {
            status: step.status,
            body,
        })
    }

    fn answer_stream(
        &self,
        request: &RunRequest<'_>,
    ) -> Result<StreamResponse<ScriptError>, ScriptError> {
        self.record("POST", request)?;
        let step = self.next()?;
        if matches!(step.fail, Some(Fail::Transport)) {
            return Err(ScriptError::Transport);
        }
        let body = match step.fail {
            Some(Fail::Read) => StreamBody::ReadFailed(ScriptError::Read),
            _ => StreamBody::Chunks(chunks_of(&step).map_err(ScriptError::Record)?),
        };
        Ok(StreamResponse {
            status: step.status,
            content_type: step.content_type.unwrap_or_default(),
            body,
        })
    }
}

fn chunks_of(step: &Step) -> Result<Vec<Vec<u8>>, serde_json::Error> {
    let sse = step
        .content_type
        .as_deref()
        .is_some_and(|value| value.to_lowercase().contains("text/event-stream"));
    if sse {
        return Ok(step
            .lines
            .iter()
            .map(|line| format!("{line}\n").into_bytes())
            .collect());
    }
    Ok(vec![serde_json::to_vec(&step.body)?])
}

impl TokenTransport for &Script {
    type Error = ScriptError;

    fn post(
        &self,
        _request: &TokenRequest,
    ) -> impl Future<Output = Result<TokenHttpResponse, ScriptError>> + Send {
        let answer = self.next().and_then(|step| {
            if matches!(step.fail, Some(Fail::Transport)) {
                return Err(ScriptError::Transport);
            }
            let body = serde_json::to_vec(&step.body).map_err(ScriptError::Record)?;
            Ok(TokenHttpResponse {
                status: step.status,
                body: SecretSlice::from(body),
            })
        });
        ready(answer)
    }
}

impl RunTransport for &Script {
    type Error = ScriptError;

    fn post(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, ScriptError>> + Send {
        ready(self.answer_json("POST", request))
    }

    fn get(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, ScriptError>> + Send {
        ready(self.answer_json("GET", request))
    }

    fn post_stream(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<StreamResponse<ScriptError>, ScriptError>> + Send {
        ready(self.answer_stream(request))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        let milliseconds = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(milliseconds);
        ready(())
    }
}

pub(super) struct Clock;

impl TokenClock for Clock {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn unix_seconds(&self) -> i64 {
        1_700_000_000
    }
}

#[derive(Default)]
pub(super) struct Source {
    next: u128,
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
