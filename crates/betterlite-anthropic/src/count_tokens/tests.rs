#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::anthropic_count_tokens::DEFAULT_BETA_VERSION;
use betterlite_config::sections::routing_timeouts;
use betterlite_config::types::LiteLlmParams;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::{HeaderMap, Method};
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::count_tokens::handler::{AnthropicCountTokensHandler, CountTokensTransport};

mod cases;
mod corpus;
mod invariants;

pub(crate) const MODEL: &str = "claude-3-5-sonnet-20241022";
pub(crate) const REQUEST_MODEL: &str = "requested-model";

type Replies = VecDeque<Option<(u16, Vec<u8>)>>;

#[derive(Debug, Clone)]
pub(crate) struct Sent {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub deadline: Duration,
}

pub(crate) struct Script {
    replies: Mutex<Replies>,
    sent: Mutex<Vec<Sent>>,
}

impl Debug for Script {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

impl Script {
    pub(crate) fn replying(status: u16, payload: &[u8]) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from([Some((status, payload.to_vec()))])),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn replying_with(replies: &[(u16, &[u8])]) -> Self {
        let scripted = replies
            .iter()
            .map(|(status, payload)| Some((*status, payload.to_vec())))
            .collect();
        Self {
            replies: Mutex::new(scripted),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            replies: Mutex::new(VecDeque::from([None])),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn request(&self) -> Sent {
        let mut requests = self.requests();
        assert_eq!(requests.len(), 1);
        requests.remove(0)
    }

    fn next_reply(&self) -> Option<(u16, Vec<u8>)> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten()
    }

    fn record(&self, request: OutboundRequest) {
        let OutboundRequest {
            method,
            url,
            headers,
            body,
            deadline,
            ..
        } = request;
        let payload = match body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        let pairs = headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method,
                url: url.to_string(),
                headers: pairs,
                body: payload,
                deadline,
            });
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        let reply = self.next_reply();
        ready(
            reply
                .ok_or(TransportError::Closed)
                .map(|(status, payload)| UpstreamResponse {
                    status,
                    headers: HeaderMap::new(),
                    body: payload,
                }),
        )
    }
}

pub(crate) fn header<'sent>(sent: &'sent Sent, name: &str) -> Option<&'sent str> {
    let value = sent.headers.iter().find(|(present, _)| present == name)?;
    Some(value.1.as_str())
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn user_messages() -> Json {
    json(r#"[{"role":"user","content":[{"type":"text","text":"hi"}],"extra":true}]"#)
}

pub(crate) fn handler(transport: Script) -> AnthropicCountTokensHandler<Script> {
    AnthropicCountTokensHandler {
        transport,
        request_timeout: b198_request_timeout(),
        beta_version: DEFAULT_BETA_VERSION.to_owned(),
    }
}

pub(crate) fn beta_handler(
    script: Script,
    beta_version: &str,
) -> AnthropicCountTokensHandler<Script> {
    AnthropicCountTokensHandler {
        transport: script,
        request_timeout: b198_request_timeout(),
        beta_version: beta_version.to_owned(),
    }
}

pub(crate) fn b198_request_timeout() -> Duration {
    let settings = routing_timeouts::Settings::default();
    Duration::try_from_secs_f64(settings.global.seconds).unwrap()
}

pub(crate) fn deployment(api_key: Option<&str>) -> LiteLlmParams {
    LiteLlmParams {
        ssl_verify: None,
        model: None,
        api_base: None,
        api_key: api_key.map(secret),
        api_version: None,
        timeout: None,
        stream_timeout: None,
        max_retries: None,
        rpm: None,
        tpm: None,
        weight: None,
    }
}

pub(crate) fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Logs {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(crate) fn subscriber(logs: Logs) -> impl Subscriber + Send + Sync {
    tracing_subscriber::registry().with(logs)
}
