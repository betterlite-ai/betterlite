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

pub(super) const MODEL: &str = "black_forest_labs/flux-pro-1.1";
pub(super) const PROMPT: &str = "a cat";
pub(super) const BASE: &str = "https://api.bfl.ai";
pub(super) const SUBMITTED: &str =
    r#"{"id":"job-1","polling_url":"https://gateway.bfl.ai/v1/get_result?id=1"}"#;
pub(super) const READY: &str =
    r#"{"status":"Ready","result":{"sample":"https://delivery.test/a.png"}}"#;

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: &'static str,
}

pub(super) fn reply(status: u16, body: &'static str) -> Reply {
    Reply { status, body }
}

impl Reply {
    pub(super) fn response(&self) -> UpstreamResponse {
        UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.as_bytes().to_vec(),
        }
    }
}

#[derive(Clone)]
pub(super) struct RecordedPost {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Json,
    pub(super) timeout: Option<Duration>,
}

pub(super) struct RecordedGet {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
}

#[derive(Clone, Copy, Default)]
pub(super) enum PostMode {
    #[default]
    Answer,
    Hang,
    Fail,
}

pub(super) struct Script {
    pub(super) submission: Reply,
    pub(super) polls: Mutex<VecDeque<Reply>>,
    pub(super) posts: Mutex<Vec<RecordedPost>>,
    pub(super) gets: Mutex<Vec<RecordedGet>>,
    pub(super) post_mode: PostMode,
    pub(super) hang_get: bool,
    pub(super) cancelled: Arc<AtomicBool>,
}

impl Script {
    pub(super) fn new(submission: Reply, polls: Vec<Reply>) -> Self {
        Self {
            submission,
            polls: Mutex::new(VecDeque::from(polls)),
            posts: Mutex::default(),
            gets: Mutex::default(),
            post_mode: PostMode::Answer,
            hang_get: false,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn ready() -> Self {
        Self::new(reply(200, SUBMITTED), vec![reply(200, READY)])
    }

    pub(super) fn last_post(&self) -> RecordedPost {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
            .expect("a submission is recorded")
    }

    pub(super) fn post_count(&self) -> usize {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
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
        if matches!(self.post_mode, PostMode::Hang) {
            let _keep = Token(Arc::clone(&self.cancelled));
            return pending::<Result<UpstreamResponse, SubmitError>>().await;
        }
        if matches!(self.post_mode, PostMode::Fail) {
            return Err(SubmitError::Transport(TransportError::Closed));
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

pub(super) struct StepClock {
    millis: AtomicU64,
    step: u64,
}

impl StepClock {
    pub(super) fn stepping(step: u64) -> Self {
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

pub(super) struct Recorder<'clock> {
    pub(super) clock: &'clock StepClock,
    pub(super) slept: Mutex<Vec<Duration>>,
}

impl<'clock> Recorder<'clock> {
    pub(super) fn new(clock: &'clock StepClock) -> Self {
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
pub(super) struct Sink(pub(super) Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

pub(super) fn logging_state() -> (Redactor, LoggingState) {
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

pub(super) fn logging_env<'setup>(
    redactor: &'setup Redactor,
    sink: &'setup Sink,
) -> PrePostEnv<'setup> {
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

pub(super) fn poll_once<F>(future: Pin<&mut F>) -> Poll<F::Output>
where
    F: Future,
{
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.poll(&mut context)
}
