use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::RangeInclusive;
use std::time::Duration;

use betterlite_a2a::completion_bridge::EventSource;
use betterlite_a2a::pydantic_ai_handler::{HandlerError, PydanticAIHandler};
use betterlite_a2a::pydantic_ai_provider::{ProviderCall, ProviderError, PydanticAIProviderConfig};
use betterlite_a2a::pydantic_ai_submit::error::SubmitError;
use betterlite_a2a::pydantic_ai_submit::{AgentRequest, AgentResponse, AgentTransport};
use betterlite_a2a::pydantic_ai_transformation::fake_streaming::events::StreamEvent;
use betterlite_a2a::pydantic_ai_transformation::send::{
    ProjectedMessage, PydanticAITransformation, ReplayedStream,
};
use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
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

#[derive(Clone, Deserialize)]
struct Reply {
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    body: AgentJson,
    #[serde(default)]
    raises: bool,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    mode: Mode,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    params: AgentJson,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    kwargs: BTreeMap<String, AgentJson>,
    #[serde(default)]
    responses: Vec<Reply>,
}

#[derive(Debug, Error)]
enum ScriptError {
    #[error("the scripted transport failed")]
    Failed,
    #[error("the request cannot be recorded")]
    Unrecordable(#[source] serde_json::Error),
}

#[derive(Serialize)]
struct Recorded {
    url: String,
    body: AgentJson,
    headers: BTreeMap<String, String>,
    timeout: f64,
}

struct Script {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
    pauses: Mutex<Vec<u64>>,
}

impl AgentTransport for Script {
    type Error = ScriptError;

    async fn post(&self, request: &AgentRequest<'_>) -> Result<AgentResponse, ScriptError> {
        let body = serde_json::to_value(request.body).map_err(ScriptError::Unrecordable)?;
        self.requests.lock().await.push(Recorded {
            url: request.url.to_owned(),
            body,
            headers: request.headers.clone(),
            timeout: request.timeout.as_secs_f64(),
        });
        let reply = self
            .replies
            .lock()
            .await
            .pop_front()
            .ok_or(ScriptError::Failed)?;
        if reply.raises {
            return Err(ScriptError::Failed);
        }
        Ok(AgentResponse {
            status: reply.status.unwrap_or(200),
            body: reply.body,
        })
    }

    async fn pause(&self, duration: Duration) {
        let milliseconds = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.pauses.lock().await.push(milliseconds);
    }
}

struct Source {
    ids: RangeInclusive<u128>,
}

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        Uuid::from_u128(self.ids.next().unwrap_or_default())
    }

    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}

#[derive(Serialize)]
struct Step<'event> {
    event: &'event StreamEvent,
    delay_ms: u64,
}

#[derive(Serialize)]
struct Failure {
    kind: &'static str,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Ok(AgentJson),
    Error(Failure),
}

#[derive(Serialize)]
struct Output {
    outcome: Outcome,
    requests: Vec<Recorded>,
    sleeps: Vec<u64>,
}

fn classify(error: &ProviderError<ScriptError>) -> Failure {
    let (kind, message) = match error {
        ProviderError::Handler(HandlerError::ApiBaseRequired) => {
            ("api_base_required", String::new())
        }
        ProviderError::Handler(HandlerError::Submit(timeout @ SubmitError::PollTimeout { .. })) => {
            ("timeout", timeout.to_string())
        }
        ProviderError::Handler(HandlerError::Submit(ended @ SubmitError::TaskEnded { .. })) => {
            ("task_ended", ended.to_string())
        }
        ProviderError::Kwargs(_) | ProviderError::Handler(HandlerError::Submit(_)) => {
            ("failed", String::new())
        }
    };
    Failure { kind, message }
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
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

#[derive(Debug, Error)]
enum CallError {
    #[error(transparent)]
    Provider(#[from] ProviderError<ScriptError>),
    #[error("the answer cannot be serialized")]
    Serialize(#[from] serde_json::Error),
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

async fn call(
    provider: &mut PydanticAIProviderConfig<Script, Source>,
    input: &Input,
) -> Result<AgentJson, CallError> {
    let request = ProviderCall {
        request_id: input.request_id.as_deref().unwrap_or("request"),
        params: &input.params,
        api_base: input.api_base.as_deref(),
        kwargs: &input.kwargs,
    };
    if input.mode == Mode::NonStreaming {
        let event = provider.handle::<ProjectedMessage>(&request).await?;
        return Ok(serde_json::to_value(event)?);
    }
    let simulated = provider.handle::<ReplayedStream>(&request).await?;
    let steps: Vec<_> = simulated
        .iter()
        .map(|step| Step {
            event: &step.event,
            delay_ms: u64::try_from(step.delay_after.as_millis()).unwrap_or(u64::MAX),
        })
        .collect();
    Ok(serde_json::to_value(steps)?)
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        let script = Script {
            replies: Mutex::new(input.responses.iter().cloned().collect()),
            requests: Mutex::new(Vec::new()),
            pauses: Mutex::new(Vec::new()),
        };
        let mut provider = PydanticAIProviderConfig {
            handler: PydanticAIHandler {
                transformation: PydanticAITransformation {
                    transport: script,
                    source: Source { ids: 1..=99 },
                },
            },
        };
        let outcome = match runtime.block_on(call(&mut provider, input)) {
            Ok(value) => Outcome::Ok(value),
            Err(CallError::Provider(error)) => Outcome::Error(classify(&error)),
            Err(error @ CallError::Serialize(_)) => {
                return Err(helper_error("SerializationError", error));
            }
        };
        let transport = provider.handler.transformation.transport;
        let output = Output {
            outcome,
            requests: transport.requests.into_inner(),
            sleeps: transport.pauses.into_inner(),
        };
        let mut value = serde_json::to_value(output)
            .map_err(|error| helper_error("SerializationError", error))?;
        normalize(&mut value, &mut HashMap::new());
        Ok(value)
    }
}
