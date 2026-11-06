use std::collections::{BTreeMap, VecDeque};
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::runway::tasks::TaskKind;
use betterlite_core::api::runway::tasks::error::RunwayTaskError;
use betterlite_core::api::runway::tasks::poll::{PollClock, TaskPoller, TaskTransport};
use betterlite_core::api::runway::tasks::settings::RunwaySettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use url::Url;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
pub(crate) struct Headers {
    kind: TaskKind,
    environment: BTreeMap<String, String>,
    #[serde(rename = "headers")]
    caller: BTreeMap<String, String>,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Endpoint {
    kind: TaskKind,
    environment: BTreeMap<String, String>,
    api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Poll {
    kind: TaskKind,
    task_id: String,
    api_base: String,
    timeout_seconds: u64,
    step_millis: u64,
    replies: Vec<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Headers(Headers),
    Endpoint(Endpoint),
    Poll(Poll),
}

struct Script {
    replies: Mutex<VecDeque<(u16, String)>>,
    urls: Mutex<Vec<String>>,
}

impl TaskTransport for Script {
    fn get(
        &self,
        url: Url,
        _headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_string());
        let (status, body) = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or((500, String::new()));
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

struct Clock {
    millis: AtomicU64,
    step: u64,
    pauses: Mutex<Vec<u64>>,
}

impl PollClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration.as_secs());
        ready(())
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RunwayTasksError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn describe(error: &RunwayTaskError) -> serde_json::Value {
    match error {
        RunwayTaskError::TimedOut { .. } => {
            json!({"error": "timeout", "message": error.to_string()})
        }
        RunwayTaskError::Failed { .. }
        | RunwayTaskError::Cancelled { .. }
        | RunwayTaskError::UnknownStatus { .. } => {
            json!({"error": "task", "message": error.to_string()})
        }
        RunwayTaskError::HttpStatus { status } => json!({"error": "http_status", "status": status}),
        RunwayTaskError::Decode { .. } | RunwayTaskError::TrailingBytes(_) => {
            json!({"error": "decode"})
        }
        RunwayTaskError::Environment { .. }
        | RunwayTaskError::MissingCredentials
        | RunwayTaskError::InvalidHeader(_)
        | RunwayTaskError::TaskUrl(_)
        | RunwayTaskError::TaskId(_)
        | RunwayTaskError::Transport(_)
        | RunwayTaskError::TooLarge { .. } => {
            json!({"error": "other", "message": error.to_string()})
        }
    }
}

fn headers(input: &Headers) -> HelperResult {
    let settings = match RunwaySettings::load(&input.environment) {
        Ok(settings) => settings,
        Err(error) => return Ok(describe(&error)),
    };
    let mut caller = HeaderMap::new();
    for (name, value) in &input.caller {
        let header = HeaderName::try_from(name.as_str()).map_err(failure)?;
        caller.insert(
            header,
            HeaderValue::try_from(value.as_str()).map_err(failure)?,
        );
    }
    let api_key = input.api_key.clone().map(SecretString::from);
    match settings.authenticate(input.kind, &caller, api_key.as_ref()) {
        Ok(sent) => {
            let flat: BTreeMap<&str, &str> = sent
                .iter()
                .filter_map(|(name, value)| Some((name.as_str(), value.to_str().ok()?)))
                .collect();
            Ok(json!({"headers": flat}))
        }
        Err(error) => Ok(describe(&error)),
    }
}

fn poll(input: &Poll) -> HelperResult {
    let transport = Script {
        replies: Mutex::new(
            input
                .replies
                .iter()
                .map(|reply| (reply.status, reply.body.clone()))
                .collect(),
        ),
        urls: Mutex::default(),
    };
    let clock = Clock {
        millis: AtomicU64::new(0),
        step: input.step_millis,
        pauses: Mutex::default(),
    };
    let poller = TaskPoller {
        transport: &transport,
        clock: &clock,
        kind: input.kind,
        timeout_seconds: input.timeout_seconds,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let outcome = runtime.block_on(poller.poll(&input.api_base, &input.task_id, &HeaderMap::new()));
    let answer = match outcome {
        Ok(task) => json!({"task": {"id": task.id, "status": task.status}}),
        Err(error) => describe(&error),
    };
    Ok(json!({
        "outcome": answer,
        "gets": *transport.urls.lock().unwrap_or_else(PoisonError::into_inner),
        "sleeps": *clock.pauses.lock().unwrap_or_else(PoisonError::into_inner),
    }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Headers(request) => headers(request),
            Input::Endpoint(request) => {
                let settings = RunwaySettings::load(&request.environment).map_err(failure)?;
                Ok(json!({"url": settings.endpoint(request.kind, request.api_base.as_deref())}))
            }
            Input::Poll(request) => poll(request),
        }
    }
}
