use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::bfl::generation_call::submit::{
    SubmitError, SubmitRequest, SubmitTransport,
};
use betterlite_core::api::bfl::poll::{PollClock, PollSleeper, PollTransport};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};
use url::Url;

use super::input::{ClockSettings, Reply};

// httpx fills these from its own client defaults and from the encoded body, so both sides record them as plumbing
const TRANSPORT_HEADERS: [&str; 6] = [
    "accept",
    "accept-encoding",
    "connection",
    "content-length",
    "host",
    "user-agent",
];

#[derive(Default)]
pub(super) struct Effects(pub(super) Mutex<Vec<Value>>);

impl Effects {
    pub(super) fn record(&self, effect: Value) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }
}

pub(super) struct Script<'effects> {
    pub(super) submission: Reply,
    pub(super) polls: Mutex<VecDeque<Reply>>,
    pub(super) effects: &'effects Effects,
}

impl Script<'_> {
    // the last scripted poll repeats, so a job that stays Pending only needs one reply
    fn next_reply(&self) -> Reply {
        let mut polls = self.polls.lock().unwrap_or_else(PoisonError::into_inner);
        let reply = if polls.len() > 1 {
            polls.pop_front()
        } else {
            polls.front().cloned()
        };
        reply.unwrap_or_else(|| self.submission.clone())
    }
}

impl SubmitTransport for Script<'_> {
    fn post(
        &self,
        request: SubmitRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, SubmitError>> + Send {
        self.effects.record(json!({
            "effect": "post",
            "url": request.url.to_string(),
            "headers": header_object(&request.headers),
            "body": request.body.clone(),
            "timeout_millis": request
                .timeout
                .map(|timeout| u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
        }));
        ready(Ok(response(&self.submission)))
    }
}

impl PollTransport for Script<'_> {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.effects.record(json!({
            "effect": "get",
            "url": url.to_string(),
            "headers": header_object(&headers),
        }));
        ready(Ok(response(&self.next_reply())))
    }
}

pub(super) struct Timeline<'effects> {
    millis: AtomicU64,
    tick: u64,
    advance: u64,
    effects: &'effects Effects,
}

impl<'effects> Timeline<'effects> {
    pub(super) fn new(settings: &ClockSettings, effects: &'effects Effects) -> Self {
        Self {
            millis: AtomicU64::new(settings.start),
            tick: settings.tick,
            advance: settings.sleep_advance,
            effects,
        }
    }
}

impl PollClock for Timeline<'_> {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.tick, Ordering::SeqCst))
    }
}

// a scripted sleep records itself and moves the fake clock, so the poll's next deadline check sees the advance
impl PollSleeper for Timeline<'_> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.effects.record(json!({
            "effect": "sleep",
            "millis": u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        }));
        self.millis.fetch_add(self.advance, Ordering::SeqCst);
        ready(())
    }
}

fn header_object(headers: &HeaderMap) -> Value {
    let mut fields = Map::new();
    for (name, value) in headers {
        if TRANSPORT_HEADERS.contains(&name.as_str()) {
            continue;
        }
        fields.insert(name.as_str().to_owned(), Value::String(header_text(value)));
    }
    Value::Object(fields)
}

fn header_text(value: &HeaderValue) -> String {
    value.to_str().map_or_else(
        |_| format!("{} bytes that are not UTF-8", value.as_bytes().len()),
        str::to_owned,
    )
}

fn response(reply: &Reply) -> UpstreamResponse {
    UpstreamResponse {
        status: reply.status,
        headers: HeaderMap::new(),
        body: reply.body.as_bytes().to_vec(),
    }
}
