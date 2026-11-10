use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_config::sections::routing_timeouts::Settings as TimeoutSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::input::{Reply, failure};

use crate::helpers::HelperError;

pub(crate) const COUNT_HEADERS: [&str; 6] = [
    "content-type",
    "x-api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
    "anthropic-dangerous-direct-browser-access",
];

#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
    pub(crate) timeout: f64,
}

pub(crate) struct Script {
    reply: Option<(u16, Vec<u8>)>,
    sent: Mutex<Vec<Seen>>,
}

impl Debug for Script {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

impl Script {
    pub(crate) fn new(reply: Option<&Reply>) -> Self {
        Self {
            reply: reply.map(|found| (found.status, found.body.clone().into_bytes())),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Seen> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: OutboundRequest) {
        let OutboundRequest {
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
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
            .collect();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                url: url.to_string(),
                headers: pairs,
                body: payload,
                timeout: deadline.as_secs_f64(),
            });
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(
            self.reply
                .clone()
                .ok_or(TransportError::Closed)
                .map(|(status, payload)| UpstreamResponse {
                    status,
                    headers: HeaderMap::new(),
                    body: payload,
                }),
        )
    }
}

pub(crate) fn request_json(seen: &Seen) -> Value {
    let body = serde_json::from_slice::<Value>(&seen.body).unwrap_or(Value::Null);
    json!({
        "url": seen.url,
        "timeout": seen.timeout,
        "headers": filtered_headers(&seen.headers),
        "body": body,
    })
}

pub(crate) fn filtered_headers(headers: &BTreeMap<String, String>) -> Value {
    let mut fields = serde_json::Map::new();
    for name in COUNT_HEADERS {
        let value = headers
            .get(name)
            .map_or(Value::Null, |found| Value::String(found.clone()));
        fields.insert(name.to_owned(), value);
    }
    Value::Object(fields)
}

pub(crate) fn request_timeout(override_seconds: Option<f64>) -> Result<Duration, Box<HelperError>> {
    let seconds = override_seconds.unwrap_or_else(|| {
        let settings = TimeoutSettings::default();
        settings.global.seconds
    });
    Duration::try_from_secs_f64(seconds).map_err(|error| failure(&error))
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
    pub(crate) fn entries(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|message| warning_entry(message))
            .collect()
    }
}

fn warning_entry(message: &str) -> Value {
    let (subject, detail) = message.split_once(": ").unwrap_or((message, ""));
    let status = detail
        .strip_prefix("status=")
        .and_then(|rest| rest.split(',').next())
        .and_then(|number| number.parse::<u16>().ok());
    json!({"subject": subject, "status": status})
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
