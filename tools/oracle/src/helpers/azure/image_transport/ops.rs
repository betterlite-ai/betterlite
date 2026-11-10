use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use betterlite_azure::image_transport::error::{FailureMessage, ImageTransportError};
use betterlite_azure::image_transport::json::{JsonMap, JsonValue};
use betterlite_azure::image_transport::request::{
    Call, CallStyle, Connection, ImageClientConstructor, PollingSettings, TimeoutPhases, execute,
};
use betterlite_azure::image_transport::{
    ImageClock, ImageReply, ImageRequest, ImageResponse, ImageTransport,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::helpers::{HelperError, HelperResult};

use super::input::{Input, Reply, Step};

pub(crate) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(&error))?;
    runtime.block_on(async {
        let mut steps = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            steps.push(run_step(step).await?);
        }
        Ok(Value::Array(steps))
    })
}

async fn run_step(step: &Step) -> HelperResult {
    let clock = Arc::new(FixtureClock::default());
    let state = Arc::new(Fixture {
        replies: Mutex::new(VecDeque::from(step.replies.clone())),
        calls: Mutex::default(),
        constructed: Mutex::default(),
        clock: Arc::clone(&clock),
    });
    let constructor = Constructor {
        state: Arc::clone(&state),
    };
    let connection = Connection {
        client: step.supplied_client.then(|| FixtureTransport {
            state: Arc::clone(&state),
        }),
        timeout: step.timeout,
    };
    let settings = PollingSettings {
        timeout: polling_timeout(&step.environment)?,
        style: if step.is_async {
            CallStyle::Async
        } else {
            CallStyle::Sync
        },
    };
    let mut data = step.data.clone();
    let call = Call {
        api_base: &step.api_base,
        api_version: &step.api_version,
        deployment_name: step.deployment_name.as_deref(),
        data: &mut data,
        headers: header_map(&step.headers)?,
    };
    let outcome = execute(&constructor, connection, &*clock, settings, call).await;
    Ok(step_value(&state, &clock, &data, &outcome))
}

fn polling_timeout(environment: &BTreeMap<String, String>) -> Result<Duration, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .map(|config| {
        config
            .sections
            .azure_image_transport
            .operation_polling_timeout
    })
    .map_err(|error| failure(&error))
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, text) in headers {
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|error| failure(&error))?;
        let value = HeaderValue::from_str(text).map_err(|error| failure(&error))?;
        map.insert(header, value);
    }
    Ok(map)
}

fn failure(message: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ImageTransportError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

struct Fixture {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<RecordedCall>>,
    constructed: Mutex<Vec<TimeoutPhases>>,
    clock: Arc<FixtureClock>,
}

struct RecordedCall {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: Option<Value>,
}

#[derive(Clone)]
struct FixtureTransport {
    state: Arc<Fixture>,
}

impl ImageTransport for FixtureTransport {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        let body = (!request.body.is_empty())
            .then(|| serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null));
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                method: request.method.to_string(),
                url: request.url.clone(),
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().to_owned(),
                            value.to_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect(),
                body,
            });
        let Some(reply) = self
            .state
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        else {
            return ready(Err(TransportError::Closed));
        };
        if reply.delay > 0 {
            self.state
                .clock
                .elapsed
                .fetch_add(reply.delay, Ordering::SeqCst);
        }
        ready(Ok(ImageReply {
            status: reply.status,
            headers: reply_headers(&reply.headers),
            body: reply.body.into_bytes(),
        }))
    }
}

fn reply_headers(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(header), Ok(parsed)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.insert(header, parsed);
        }
    }
    map
}

#[derive(Default)]
struct FixtureClock {
    elapsed: AtomicU64,
    sleeps: Mutex<Vec<i64>>,
}

impl ImageClock for FixtureClock {
    fn now(&self) -> Duration {
        Duration::from_secs(self.elapsed.load(Ordering::SeqCst))
    }

    fn pause(&self, seconds: i64) -> impl Future<Output = ()> + Send {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seconds);
        if let Ok(positive) = u64::try_from(seconds) {
            self.elapsed.fetch_add(positive, Ordering::SeqCst);
        }
        ready(())
    }
}

struct Constructor {
    state: Arc<Fixture>,
}

impl ImageClientConstructor for Constructor {
    type Client = FixtureTransport;

    fn construct(&self, timeout: TimeoutPhases) -> FixtureTransport {
        self.state
            .constructed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(timeout);
        FixtureTransport {
            state: Arc::clone(&self.state),
        }
    }
}

fn step_value(
    state: &Fixture,
    clock: &FixtureClock,
    data: &JsonMap,
    outcome: &Result<ImageResponse, ImageTransportError>,
) -> Value {
    let (kind, status, headers, body, request, message_kind) = match outcome {
        Ok(response) => (
            "response",
            json!(response.status),
            headers_value(&response.headers),
            body_value(&response.body),
            json!({"method": response.request.method.as_str(), "url": response.request.url}),
            Value::Null,
        ),
        Err(error) => (
            "error",
            json!(error.status_code()),
            Value::Null,
            failure_body(error),
            Value::Null,
            failure_kind(error),
        ),
    };
    let sleeps = clock
        .sleeps
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({
        "kind": kind,
        "status": status,
        "headers": headers,
        "body": body,
        "request": request,
        "message_kind": message_kind,
        "data": serde_json::to_value(data).unwrap_or(Value::Null),
        "calls": calls_value(state),
        "sleeps": sleeps,
        "constructed": constructed_value(state),
    })
}

fn failure_body(error: &ImageTransportError) -> Value {
    match failure_detail(error) {
        Some((body, _kind)) => serde_json::to_value(body).unwrap_or(Value::Null),
        None => Value::Null,
    }
}

fn failure_kind(error: &ImageTransportError) -> Value {
    match failure_detail(error) {
        Some((_body, kind)) => Value::String(kind.to_owned()),
        None => Value::Null,
    }
}

fn failure_detail(error: &ImageTransportError) -> Option<(&JsonValue, &'static str)> {
    match error {
        ImageTransportError::OperationFailed { body, .. } => {
            Some((body, FailureMessage::of(body).as_str()))
        }
        ImageTransportError::RejectedPollingUrl { .. }
        | ImageTransportError::MissingOperationLocation { .. }
        | ImageTransportError::MissingStatusInitial
        | ImageTransportError::PollingTimeout
        | ImageTransportError::Decode { .. }
        | ImageTransportError::MissingStatus
        | ImageTransportError::MissingResult
        | ImageTransportError::RetryAfter { .. }
        | ImageTransportError::NegativeSleep { .. }
        | ImageTransportError::Url { .. }
        | ImageTransportError::Encode { .. }
        | ImageTransportError::Transport(_) => None,
    }
}

fn headers_value(headers: &HeaderMap) -> Value {
    let mut value = serde_json::Map::new();
    for (name, header) in headers {
        if let Ok(text) = header.to_str() {
            value.insert(name.as_str().to_owned(), Value::String(text.to_owned()));
        }
    }
    Value::Object(value)
}

fn body_value(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or_else(|_| {
        std::str::from_utf8(body).map_or(Value::Null, |text| Value::String(text.to_owned()))
    })
}

fn calls_value(state: &Fixture) -> Value {
    Value::Array(
        state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|call| {
                json!({
                    "method": call.method,
                    "url": call.url,
                    "headers": call.headers,
                    "body": call.body,
                    "timeout": Value::Null,
                })
            })
            .collect(),
    )
}

fn constructed_value(state: &Fixture) -> Value {
    Value::Array(
        state
            .constructed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|phases| {
                json!({
                    "connect": phases.connect,
                    "read": phases.read,
                    "write": phases.write,
                    "pool": phases.pool,
                })
            })
            .collect(),
    )
}
