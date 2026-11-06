use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Display;
use std::future::{Future, ready};
use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_a2a::completion_bridge::EventSource;
use betterlite_a2a::completion_bridge::events::Event;
use betterlite_a2a::watsonx_credentials::bearer::{
    BearerTokens, CredentialsError, TokenClock, TokenHttpResponse, TokenRequest, TokenTransport,
};
use betterlite_a2a::watsonx_credentials::token_cache::BoundedTokenCache;
use betterlite_a2a::watsonx_run::config::{ConfigCall, ConfigError, WatsonxRunConfig};
use betterlite_a2a::watsonx_run::handler::{RunError, RunHandler};
use betterlite_a2a::watsonx_run::poll::PollError;
use betterlite_a2a::watsonx_run::run_api::ApiError;
use betterlite_a2a::watsonx_run::transport::{
    RunRequest, RunResponse, RunTransport, StreamBody, StreamResponse,
};
use betterlite_a2a::watsonx_transform::extract::WxoSendParams;
use betterlite_a2a::watsonx_transform::fake_streaming::WxoStreamResult;
use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use secrecy::SecretSlice;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    NonStreaming,
    Streaming,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Fail {
    Transport,
    Read,
}

#[derive(Clone, Deserialize)]
struct Step {
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    body: AgentJson,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    lines: Vec<String>,
    #[serde(default)]
    fail: Option<Fail>,
    #[serde(default)]
    repeat: bool,
}

const fn default_status() -> u16 {
    200
}

fn default_request_id() -> String {
    "request".to_owned()
}

#[derive(Deserialize)]
pub(crate) struct Input {
    mode: Mode,
    #[serde(default = "default_request_id")]
    request_id: String,
    params: WxoSendParams,
    #[serde(default)]
    kwargs: BTreeMap<String, AgentJson>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Debug, Error)]
enum ScriptError {
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
struct Recorded {
    method: &'static str,
    url: String,
    headers: BTreeMap<String, String>,
    body: AgentJson,
    timeout: f64,
}

struct Script {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<Recorded>>,
    pauses: Mutex<Vec<u64>>,
}

impl Script {
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

struct Clock;

impl TokenClock for Clock {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn unix_seconds(&self) -> i64 {
        1_700_000_000
    }
}

#[derive(Default)]
struct Source {
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

#[derive(Debug, Error)]
enum CallError {
    #[error(transparent)]
    Config(#[from] ConfigError<ScriptError, ScriptError>),
    #[error("the answer cannot be serialized")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct PacedEvent<'event> {
    event: &'event Event<WxoStreamResult>,
    delay_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Ok(AgentJson),
    Error(Failure),
}

#[derive(Serialize)]
struct Failure {
    kind: &'static str,
}

#[derive(Serialize)]
struct Output {
    outcome: Outcome,
    requests: Vec<Recorded>,
    sleeps: Vec<u64>,
}

const fn credentials_kind(error: &CredentialsError<ScriptError>) -> &'static str {
    match error {
        CredentialsError::MissingUsername | CredentialsError::Encode(_) => "params_missing",
        CredentialsError::Transport(_) => "transport",
        CredentialsError::Status { .. } => "status",
        CredentialsError::Decode(_) => "decode",
    }
}

const fn api_kind(error: &ApiError<ScriptError>) -> &'static str {
    match error {
        ApiError::Transport(_) => "transport",
        ApiError::Status { .. } => "status",
        ApiError::Decode(_) => "decode",
    }
}

const fn poll_kind(error: &PollError<ScriptError>) -> &'static str {
    match error {
        PollError::Api(api) => api_kind(api),
        PollError::NoRunId => "no_run_id",
        PollError::PollTimeout { .. } => "timeout",
        PollError::NonSuccess { .. } => "non_success",
    }
}

const fn run_kind(error: &RunError<ScriptError, ScriptError>) -> &'static str {
    match error {
        RunError::Params(_) => "params_missing",
        RunError::Credentials(credentials) => credentials_kind(credentials),
        RunError::Api(api) => api_kind(api),
        RunError::Poll(poll) => poll_kind(poll),
        RunError::Sse(_) => "sse",
        RunError::Body(_) => "read",
    }
}

const fn kind(error: &CallError) -> &'static str {
    match error {
        CallError::Config(ConfigError::MissingLitellmParams) => "config_missing",
        CallError::Config(ConfigError::LitellmParams(_)) => "config_invalid",
        CallError::Config(ConfigError::Run(run)) => run_kind(run),
        CallError::Serialize(_) => "serialize",
    }
}

async fn call(
    config: &mut WatsonxRunConfig<&Script, &Script, Clock, BoundedTokenCache, Source>,
    input: &Input,
) -> Result<AgentJson, CallError> {
    let request = ConfigCall {
        request_id: &input.request_id,
        params: &input.params,
        kwargs: &input.kwargs,
    };
    if input.mode == Mode::NonStreaming {
        let event = config.handle_non_streaming(&request).await?;
        return Ok(serde_json::to_value(event)?);
    }
    let steps = config.handle_streaming(&request).await?;
    let paced = steps
        .iter()
        .map(|step| PacedEvent {
            event: &step.event,
            delay_ms: u64::try_from(step.delay_after.as_millis()).unwrap_or(u64::MAX),
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_value(paced)?)
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

fn normalize(value: &mut AgentJson, seen: &mut HashMap<String, String>) {
    match value {
        AgentJson::String(text) => {
            if text.len() == 36 && text.starts_with("00000000-0000-0000-0000-") {
                let next = format!("uuid-{}", seen.len());
                let replacement = seen.entry(text.clone()).or_insert(next);
                text.clone_from(replacement);
            }
        }
        AgentJson::Array(items) => {
            for item in items {
                normalize(item, seen);
            }
        }
        AgentJson::Object(fields) => {
            for field in fields.values_mut() {
                normalize(field, seen);
            }
        }
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Number(_) => {}
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        let script = Script {
            steps: Mutex::new(VecDeque::from(input.steps.clone())),
            requests: Mutex::new(Vec::new()),
            pauses: Mutex::new(Vec::new()),
        };
        let mut config = WatsonxRunConfig {
            handler: RunHandler {
                transport: &script,
                credentials: BearerTokens {
                    transport: &script,
                    clock: Clock,
                    store: BoundedTokenCache::new(NonZeroUsize::MIN),
                },
                source: Source::default(),
            },
        };
        let outcome = match runtime.block_on(call(&mut config, input)) {
            Ok(value) => Outcome::Ok(value),
            Err(error) => Outcome::Error(Failure { kind: kind(&error) }),
        };
        drop(config);
        let output = Output {
            outcome,
            requests: script
                .requests
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner),
            sleeps: script
                .pauses
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner),
        };
        let mut value = serde_json::to_value(output)
            .map_err(|error| helper_error("SerializationError", error))?;
        normalize(&mut value, &mut HashMap::new());
        Ok(value)
    }
}
