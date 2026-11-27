use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::runway::speech::dispatch::{
    HandlerFailure, HandlerRequest, TextToSpeechHandler,
};
use betterlite_core::api::runway::tasks::poll::{PollClock, TaskTransport};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use url::Url;

use super::inputs::ReplyInput;

pub(super) struct Script {
    replies: Mutex<VecDeque<ReplyInput>>,
    requests: Mutex<Vec<(String, HeaderMap)>>,
}

impl Script {
    pub(super) fn new(replies: &[ReplyInput]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn describe(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(url, headers)| {
                json!({
                    "url": url,
                    "headers": {
                        "authorization": header_text(headers, AUTHORIZATION.as_str()),
                        "x-runway-version": header_text(headers, "x-runway-version"),
                    },
                })
            })
            .collect()
    }
}

impl TaskTransport for Script {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_string(), headers));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ReplyInput {
                status: 500,
                headers: BTreeMap::new(),
                body: String::new(),
            });
        let headers = reply_map(&reply.headers);
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers,
            body: reply.body.into_bytes(),
        }))
    }
}

fn reply_map(values: &BTreeMap<String, String>) -> HeaderMap {
    values
        .iter()
        .filter_map(|(name, value)| {
            Some((
                HeaderName::from_bytes(name.as_bytes()).ok()?,
                HeaderValue::from_bytes(value.as_bytes()).ok()?,
            ))
        })
        .collect()
}

fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

pub(super) struct Clock {
    millis: AtomicU64,
    step: u64,
    pauses: Mutex<Vec<u64>>,
}

impl Clock {
    pub(super) fn new(step: u64) -> Self {
        Self {
            millis: AtomicU64::new(0),
            step,
            pauses: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn pauses(&self) -> Vec<u64> {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
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

#[derive(Default)]
pub(super) struct Recorder {
    calls: Vec<Value>,
}

impl Recorder {
    pub(super) fn take(&mut self) -> Value {
        self.calls.pop().unwrap_or(Value::Null)
    }
}

impl TextToSpeechHandler for Recorder {
    type Output = ();

    fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> impl Future<Output = Result<(), HandlerFailure>> + Send {
        self.calls.push(json!({
            "model": request.model,
            "input": request.input,
            "voice": request.voice,
            "optional_params": request.optional_params,
            "litellm_params": request.litellm_params,
            "api_base": request.api_base,
            "api_key": request.api_key.map(|key| key.expose_secret().to_owned()),
            "timeout_seconds": request.timeout_seconds,
            "extra_headers": request.extra_headers,
            "aspeech": request.aspeech,
            "client": null,
        }));
        ready(Ok(()))
    }
}
