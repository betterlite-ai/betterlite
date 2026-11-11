use std::collections::{BTreeMap, VecDeque};
use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_azure::image_transport::ImageResponse;
use betterlite_azure::image_transport::error::{FailureMessage, ImageTransportError};
use betterlite_azure::image_transport::json::{JsonMap, JsonValue};
use betterlite_azure::image_transport::request::{
    Call, CallStyle, Connection, PollingSettings, execute,
};
use betterlite_config::loader::ConfigLoader;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::fixture::{Constructor, Fixture, FixtureClock, FixtureTransport, lock};
use super::input::{Input, Step};

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

pub(super) fn header_map(
    headers: &BTreeMap<String, String>,
) -> Result<HeaderMap, Box<HelperError>> {
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

fn outcome_fields(
    outcome: &Result<ImageResponse, ImageTransportError>,
) -> (&'static str, Value, Value, Value, Value, Value) {
    match outcome {
        Ok(response) => (
            "response",
            json!(response.status),
            headers_value(&response.headers),
            body_value(&response.body),
            json!({"method": response.request.method.as_str(), "url": response.request.url}),
            Value::Null,
        ),
        Err(error) => {
            let detail = failure_detail(error);
            (
                "error",
                json!(error.status_code()),
                Value::Null,
                detail.map_or(Value::Null, |(body, _kind)| {
                    serde_json::to_value(body).unwrap_or(Value::Null)
                }),
                Value::Null,
                detail.map_or(Value::Null, |(_body, kind)| Value::String(kind.to_owned())),
            )
        }
    }
}

fn step_value(
    state: &Fixture,
    clock: &FixtureClock,
    data: &JsonMap,
    outcome: &Result<ImageResponse, ImageTransportError>,
) -> Value {
    let (kind, status, headers, body, request, message_kind) = outcome_fields(outcome);
    let calls: Vec<Value> = lock(&state.calls)
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
        .collect();
    let constructed: Vec<Value> = lock(&state.constructed)
        .iter()
        .map(|phases| {
            json!({
                "connect": phases.connect,
                "read": phases.read,
                "write": phases.write,
                "pool": phases.pool,
            })
        })
        .collect();
    let sleeps = lock(&clock.sleeps).clone();
    json!({
        "kind": kind,
        "status": status,
        "headers": headers,
        "body": body,
        "request": request,
        "message_kind": message_kind,
        "data": serde_json::to_value(data).unwrap_or(Value::Null),
        "calls": calls,
        "sleeps": sleeps,
        "constructed": constructed,
    })
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
        | ImageTransportError::Decode(_)
        | ImageTransportError::MissingStatus
        | ImageTransportError::MissingResult
        | ImageTransportError::RetryAfter { .. }
        | ImageTransportError::NegativeSleep { .. }
        | ImageTransportError::Url(_)
        | ImageTransportError::Encode(_)
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
