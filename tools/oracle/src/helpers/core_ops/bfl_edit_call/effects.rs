use std::collections::VecDeque;
use std::future::{Future, ready};
use std::mem::take;
use std::str;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::bfl::generation_call::submit::{
    SubmitError, SubmitRequest, SubmitTransport,
};
use betterlite_core::api::bfl::poll::{PollClock, PollSleeper, PollTransport};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::HeaderMap;
use serde_json::{Value, json};
use url::Url;

use super::input::{Job, Reply};

// httpx fills these from its own client defaults and from the encoded body, so both sides leave them out
const TRANSPORT_HEADERS: [&str; 6] = [
    "accept",
    "accept-encoding",
    "connection",
    "content-length",
    "host",
    "user-agent",
];

pub(super) struct Wire<'job> {
    submission: &'job Reply,
    polls: Mutex<VecDeque<Reply>>,
    effects: Mutex<Vec<Value>>,
    index: AtomicUsize,
    millis: AtomicU64,
    tick: u64,
    advance: u64,
}

impl<'job> Wire<'job> {
    pub(super) fn new(job: &'job Job) -> Self {
        Self {
            submission: &job.submission,
            polls: Mutex::new(VecDeque::from(job.polls.clone())),
            effects: Mutex::default(),
            index: AtomicUsize::new(0),
            millis: AtomicU64::new(job.clock.start_millis),
            tick: job.clock.tick_millis,
            advance: job.clock.sleep_advance_millis,
        }
    }

    pub(super) fn record(&self, effect: Value) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }

    pub(super) fn take_effects(&self) -> Vec<Value> {
        take(&mut *self.effects.lock().unwrap_or_else(PoisonError::into_inner))
    }

    // the last scripted poll repeats, so a job that stays Pending only needs one reply
    fn next_poll(&self) -> Reply {
        let polls = self.polls.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(last) = polls.len().checked_sub(1) else {
            return self.submission.clone();
        };
        let index = self.index.fetch_add(1, Ordering::SeqCst);
        polls
            .get(index.min(last))
            .cloned()
            .unwrap_or_else(|| self.submission.clone())
    }
}

impl SubmitTransport for Wire<'_> {
    async fn post(&self, request: SubmitRequest) -> Result<UpstreamResponse, SubmitError> {
        self.record(json!({
            "effect": "post",
            "url": request.url.to_string(),
            "headers": shown_headers(&request.headers),
            "body": request.body,
            "timeout_millis": request
                .timeout
                .map(|timeout| u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
        }));
        Ok(answer(self.submission))
    }
}

impl PollTransport for Wire<'_> {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        self.record(json!({
            "effect": "get",
            "url": url.to_string(),
            "headers": shown_headers(&headers),
        }));
        Ok(answer(&self.next_poll()))
    }
}

impl PollClock for Wire<'_> {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.tick, Ordering::SeqCst))
    }
}

impl PollSleeper for Wire<'_> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.record(json!({
            "effect": "sleep",
            "millis": u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        }));
        self.millis.fetch_add(self.advance, Ordering::SeqCst);
        ready(())
    }
}

fn shown_headers(headers: &HeaderMap) -> Value {
    let shown = headers
        .iter()
        .filter(|(name, _)| !TRANSPORT_HEADERS.contains(&name.as_str()))
        .map(|(name, value)| {
            let text = str::from_utf8(value.as_bytes()).unwrap_or("bytes that are not UTF-8");
            (name.as_str().to_owned(), Value::String(text.to_owned()))
        });
    Value::Object(shown.collect())
}

fn answer(reply: &Reply) -> UpstreamResponse {
    UpstreamResponse {
        status: reply.status,
        headers: HeaderMap::new(),
        body: reply.body.as_bytes().to_vec(),
    }
}
