use std::collections::{BTreeMap, VecDeque};
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_provider::oci_common::Clock;
use betterlite_provider::replicate::poll::ReplicateSleeper;
use betterlite_provider::replicate::poll::transport::{
    ReplicateRequest, ReplicateResponse, ReplicateTransport,
};
use betterlite_provider::replicate::reply::{PostCall, PostCallLogger, TokenCounter};
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::helpers::HelperError;

pub(super) const FIXED_NOW: i64 = 1_700_000_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptedResponse {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    pub(crate) body: String,
}

const fn default_status() -> u16 {
    200
}

pub(super) fn helper_error<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
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

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))
}

pub(super) fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

pub(super) fn header_map(
    entries: &BTreeMap<String, String>,
) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in entries {
        let header_name =
            HeaderName::try_from(name).map_err(|error| helper_error("InvalidHeaderName", error))?;
        let header_value = HeaderValue::try_from(value)
            .map_err(|error| helper_error("InvalidHeaderValue", error))?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

pub(crate) fn header_map_output(
    headers: &HeaderMap,
) -> Result<Map<String, Value>, Box<HelperError>> {
    let mut found = Map::new();
    for (name, value) in headers {
        let text = value
            .to_str()
            .map_err(|error| helper_error("HeaderEncoding", error))?;
        found.insert(name.as_str().to_owned(), Value::String(text.to_owned()));
    }
    Ok(found)
}

#[derive(Clone)]
pub(super) struct ScriptedTransport {
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
    requests: Arc<Mutex<Vec<ReplicateRequest>>>,
}

impl ScriptedTransport {
    pub(super) fn new(responses: Vec<ScriptedResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn requests_output(&self) -> Result<Value, Box<HelperError>> {
        let requests = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut outputs = Vec::with_capacity(requests.len());
        for request in &requests {
            outputs.push(request_output(request)?);
        }
        Ok(Value::Array(outputs))
    }
}

#[async_trait]
impl ReplicateTransport for ScriptedTransport {
    async fn send(&self, request: ReplicateRequest) -> Result<ReplicateResponse, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let next = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match next {
            Some(scripted) => Ok(ReplicateResponse {
                status: scripted.status,
                headers: HeaderMap::new(),
                body: scripted.body.into_bytes(),
            }),
            None => Err(TransportError::Closed),
        }
    }
}

fn request_output(request: &ReplicateRequest) -> Result<Value, Box<HelperError>> {
    let body = match &request.body {
        Some(bytes) => serde_json::from_slice::<Value>(bytes)
            .map_err(|error| helper_error("RequestBodyEncoding", error))?,
        None => Value::Null,
    };
    Ok(serde_json::json!({
        "method": request.method.as_str(),
        "url": request.url,
        "headers": header_map_output(&request.headers)?,
        "body": body,
    }))
}

#[derive(Clone, Default)]
pub(super) struct RecordingSleeper {
    delays: Arc<Mutex<Vec<f64>>>,
}

impl RecordingSleeper {
    pub(super) fn delays(&self) -> Vec<f64> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ReplicateSleeper for RecordingSleeper {
    async fn sleep(&self, seconds: f64) {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seconds);
    }
}

pub(super) struct FixedClock(pub(super) DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

// the count carries the model name's length, so a counter handed the wrong model shows in the usage
pub(super) struct StandInCounter;

fn stand_in_count(model: &str, size: usize) -> Result<i64, CountError> {
    let total = model
        .chars()
        .count()
        .checked_add(size)
        .ok_or(CountError::Overflow)?;
    i64::try_from(total).map_err(|_overflow| CountError::Overflow)
}

#[async_trait]
impl TokenCounter for StandInCounter {
    async fn count_messages(&self, model: &str, messages: &[Message]) -> Result<i64, CountError> {
        stand_in_count(model, messages.len())
    }

    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<i64, CountError> {
        stand_in_count(model, text.chars().count())
    }
}

pub(super) struct SilentLogger;

impl PostCallLogger for SilentLogger {
    fn post_call(&self, _call: PostCall<'_>) {}
}
