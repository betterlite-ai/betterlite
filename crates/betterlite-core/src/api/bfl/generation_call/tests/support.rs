use std::collections::VecDeque;
use std::future::{Future, pending, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use http::header::HeaderMap;
use url::Url;

use crate::api::bfl::generation_call::submit::{SubmitError, SubmitRequest, SubmitTransport};
use crate::api::bfl::poll::{PollClock, PollSleeper, PollTransport};

pub const MODEL: &str = "black_forest_labs/flux-pro-1.1";
pub const PROMPT: &str = "a cat";
pub const BASE: &str = "https://api.bfl.ai";
pub const SUBMITTED: &str =
    r#"{"id":"job-1","polling_url":"https://gateway.bfl.ai/v1/get_result?id=1"}"#;
pub const READY: &str = r#"{"status":"Ready","result":{"sample":"https://delivery.test/a.png"}}"#;

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub body: &'static str,
}

pub fn reply(status: u16, body: &'static str) -> Reply {
    Reply { status, body }
}

impl Reply {
    pub fn response(&self) -> UpstreamResponse {
        UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.as_bytes().to_vec(),
        }
    }
}

#[derive(Clone)]
pub struct RecordedPost {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Json,
    pub timeout: Option<Duration>,
}

pub struct RecordedGet {
    pub url: String,
    pub headers: HeaderMap,
}

pub struct Script {
    pub submission: Reply,
    polls: Mutex<VecDeque<Reply>>,
    pub posts: Mutex<Vec<RecordedPost>>,
    pub gets: Mutex<Vec<RecordedGet>>,
    pub hang_post: bool,
    pub hang_get: bool,
    pub cancelled: Arc<AtomicBool>,
}

impl Script {
    pub fn new(submission: Reply, polls: Vec<Reply>) -> Self {
        Self {
            submission,
            polls: Mutex::new(polls.into()),
            posts: Mutex::default(),
            gets: Mutex::default(),
            hang_post: false,
            hang_get: false,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn ready() -> Self {
        Self::new(reply(200, SUBMITTED), vec![reply(200, READY)])
    }

    pub fn last_post(&self) -> RecordedPost {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
            .expect("a submission is recorded")
    }
}

struct Token(Arc<AtomicBool>);

impl Drop for Token {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl SubmitTransport for Script {
    async fn post(&self, request: SubmitRequest) -> Result<UpstreamResponse, SubmitError> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedPost {
                url: request.url.to_string(),
                headers: request.headers.clone(),
                body: request.body.clone(),
                timeout: request.timeout,
            });
        if self.hang_post {
            let _keep = Token(Arc::clone(&self.cancelled));
            return pending::<Result<UpstreamResponse, SubmitError>>().await;
        }
        Ok(self.submission.response())
    }
}

impl PollTransport for Script {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedGet {
                url: url.to_string(),
                headers,
            });
        if self.hang_get {
            let _keep = Token(Arc::clone(&self.cancelled));
            return pending::<Result<UpstreamResponse, TransportError>>().await;
        }
        let reply = {
            let mut polls = self.polls.lock().unwrap_or_else(PoisonError::into_inner);
            polls.pop_front().unwrap_or_else(|| self.submission.clone())
        };
        Ok(reply.response())
    }
}

pub struct StepClock {
    millis: AtomicU64,
    step: u64,
}

impl StepClock {
    pub fn stepping(step: u64) -> Self {
        Self {
            millis: AtomicU64::new(0),
            step,
        }
    }
}

impl PollClock for StepClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }
}

pub struct Recorder<'clock> {
    clock: &'clock StepClock,
    pub slept: Mutex<Vec<Duration>>,
}

impl<'clock> Recorder<'clock> {
    pub fn new(clock: &'clock StepClock) -> Self {
        Self {
            clock,
            slept: Mutex::default(),
        }
    }
}

impl PollSleeper for Recorder<'_> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.slept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        let step = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.clock.millis.fetch_add(step, Ordering::SeqCst);
        ready(())
    }
}

#[derive(Default)]
pub struct Sink(pub Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

pub fn logging_state() -> (Redactor, LoggingState) {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"flux-pro-1.1","call_type":"image_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds");
    (redactor, state)
}

pub fn logging_env<'setup>(redactor: &'setup Redactor, sink: &'setup Sink) -> PrePostEnv<'setup> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses"),
    }
}

pub fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.poll(&mut context)
}
