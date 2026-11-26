#![cfg(test)]

use std::collections::VecDeque;
use std::env;
use std::future::{pending, ready};
use std::io;
use std::iter::repeat_n;
use std::process::Command;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::billing::{
    BillingDeps, PayloadInput, ProductionPayload, RunPayload, RunSettings, TinyfishRunBilling,
};
use super::error::TinyfishError;
use super::fetch::{
    RUN_API_KEY_HEADER, RunCredentials, RunFetchFuture, RunReply, RunRequest, RunTransport,
};
use super::poll::{PollClock, PollSleepFuture, PollSleeper};
use super::records::TinyfishRun;
use crate::pass_through_logging::background_tasks::BackgroundTasks;

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

// The logging object stamps this start time, so a billing test compares against the same instant.
pub(super) fn start_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: "fixture-model".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: start_time(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

pub(super) fn logging_object() -> StreamLoggingObject {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        logging_init(),
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    }
}

pub(super) fn shared_logging_object() -> Arc<StreamLoggingObject> {
    Arc::new(logging_object())
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(super) fn run(text: &str) -> TinyfishRun {
    serde_json::from_str(text).expect("a run object")
}

pub(super) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a decimal literal")
}

// The child run sees the variable in its own environment, which the parent process cannot set for the resolver.
pub(super) fn rerun_test_with_environment(test: &str, variable: &str, value: &str, marker: &str) {
    let child = Command::new(env::current_exe().expect("the test binary path"))
        .args(["--exact", test, "--nocapture"])
        .env(marker, "1")
        .env(variable, value)
        .output()
        .expect("the test binary starts");
    let stdout = str::from_utf8(&child.stdout).expect("the child output is utf8");
    assert!(child.status.success(), "{child:?}");
    assert!(stdout.contains(marker), "{child:?}");
}

pub(super) fn kwargs(entries: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    let mut collected = IndexMap::new();
    for (key, value) in entries {
        collected.insert((*key).to_owned(), value.clone());
    }
    PassThroughLoggingKwargs { entries: collected }
}

pub(super) fn details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

pub(super) fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}

pub(super) fn logged_provider(logging_obj: &StreamLoggingObject) -> Option<String> {
    logging_obj.read(&mut |state| state.custom_llm_provider.clone())?
}

pub(super) fn detail_text(logging_obj: &StreamLoggingObject, name: &str) -> Option<String> {
    match details(logging_obj).get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

pub(super) fn detail_cost(logging_obj: &StreamLoggingObject) -> Option<String> {
    match details(logging_obj).get("response_cost") {
        Some(Json::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

// The fallback paths only warn, so a captured log line is the only proof they ran.
pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let buffer = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

// The payload text a Standard response carries, which is what a billing test compares.
pub(super) fn standard_text(result: &Json) -> String {
    let Json::Object(entries) = result else {
        panic!("the standard response is an object: {result:?}");
    };
    match entries.get("response") {
        Some(Json::Text(text)) => text.clone(),
        other => panic!("the standard response carries a text: {other:?}"),
    }
}

pub(super) fn outcome_text(outcome: &HandlerOutcome) -> String {
    match outcome.result.as_ref() {
        Some(PassThroughResponse::Standard(response)) => match &response.response {
            Json::Text(text) => text.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                panic!(
                    "the standard response carries a text: {:?}",
                    response.response
                )
            }
        },
        other => panic!("the outcome carries a standard response: {other:?}"),
    }
}

// One recorded success dispatch, as the poller hands it to B3605's lower sink.
#[derive(Clone, Debug)]
pub(super) struct Dispatched {
    pub model: String,
    pub provider: Option<String>,
    pub call_id: String,
    pub cache_hit: bool,
    pub result: Json,
    pub kwargs: PassThroughLoggingKwargs,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

#[derive(Clone, Default)]
pub(super) struct RecordingDispatch(Arc<Mutex<Vec<Dispatched>>>);

impl RecordingDispatch {
    pub(super) fn calls(&self) -> Vec<Dispatched> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: &StreamSuccessCall<'_>) {
        let result = call.result.response().clone();
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Dispatched {
                model: call.state.model.clone(),
                provider: call.state.custom_llm_provider.clone(),
                call_id: call.state.litellm_call_id.clone(),
                cache_hit: call.cache_hit,
                result,
                kwargs: call.kwargs.clone(),
                start_time: call.start_time,
                end_time: call.end_time,
            });
    }
}

#[async_trait]
impl StreamSuccessDispatch for RecordingDispatch {
    async fn dispatch_success(&self, call: StreamSuccessCall<'_>) {
        self.record(&call);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedRequest {
    pub url: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
}

#[derive(Clone)]
pub(super) enum ScriptedReply {
    Run(u16, &'static str),
    Bytes(u16, &'static [u8]),
    Repeating(u16, &'static str),
    Hanging,
    Unreachable,
}

pub(super) fn unreachable(count: usize) -> Vec<ScriptedReply> {
    repeat_n(ScriptedReply::Unreachable, count).collect()
}

#[derive(Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<ScriptedReply>>,
    repeat: Mutex<Option<(u16, &'static str)>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            repeat: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next_reply(&self) -> ScriptedReply {
        let scripted = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match scripted {
            Some(ScriptedReply::Repeating(status, body)) => {
                *self.repeat.lock().unwrap_or_else(PoisonError::into_inner) = Some((status, body));
                ScriptedReply::Run(status, body)
            }
            Some(reply) => reply,
            None => match *self.repeat.lock().unwrap_or_else(PoisonError::into_inner) {
                Some((status, body)) => ScriptedReply::Run(status, body),
                None => ScriptedReply::Unreachable,
            },
        }
    }
}

impl RunTransport for ScriptedTransport {
    fn get(&self, request: RunRequest) -> RunFetchFuture<'_> {
        let api_key = request
            .headers
            .get(RUN_API_KEY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url,
                api_key,
                timeout: request.timeout,
            });
        match self.next_reply() {
            ScriptedReply::Run(status, body) => Box::pin(ready(Ok(RunReply {
                status,
                body: body.as_bytes().to_vec(),
            }))),
            ScriptedReply::Bytes(status, body) => Box::pin(ready(Ok(RunReply {
                status,
                body: body.to_vec(),
            }))),
            ScriptedReply::Hanging => Box::pin(pending()),
            ScriptedReply::Repeating(_, _) | ScriptedReply::Unreachable => {
                Box::pin(ready(Err(TransportError::Rejected(HttpError::Timeout))))
            }
        }
    }
}

pub(super) struct FixedCredentials {
    pub key: Option<SecretString>,
}

impl RunCredentials for FixedCredentials {
    fn api_key(&self) -> Option<SecretString> {
        self.key.clone()
    }
}

pub(super) struct TestClock {
    elapsed: Mutex<Duration>,
}

impl TestClock {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            elapsed: Mutex::new(Duration::ZERO),
        })
    }

    pub(super) fn advance(&self, duration: Duration) {
        let mut elapsed = self.elapsed.lock().unwrap_or_else(PoisonError::into_inner);
        *elapsed = elapsed.saturating_add(duration);
    }
}

impl PollClock for TestClock {
    fn now(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// A poll interval passes on the fake clock instead of in real time, so a deadline test stays instant.
pub(super) struct AdvancingSleeper {
    pub clock: Arc<TestClock>,
    pub sleeps: Mutex<Vec<Duration>>,
}

impl AdvancingSleeper {
    pub(super) fn new(clock: Arc<TestClock>) -> Self {
        Self {
            clock,
            sleeps: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn slept(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PollSleeper for AdvancingSleeper {
    fn sleep(&self, duration: Duration) -> PollSleepFuture<'_> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        self.clock.advance(duration);
        Box::pin(ready(()))
    }
}

pub(super) struct FixedSuccessClock;

impl SuccessClock for FixedSuccessClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2024-01-01T00:00:05+00:00").expect("a valid timestamp")
    }
}

pub(super) struct Fixture {
    pub billing: TinyfishRunBilling,
    pub transport: Arc<ScriptedTransport>,
    pub dispatch: Arc<RecordingDispatch>,
    pub sleeper: Arc<AdvancingSleeper>,
    pub tasks: TaskTracker,
}

pub(super) fn fixture(replies: Vec<ScriptedReply>) -> Fixture {
    fixture_with_credentials(replies, Some(secret("key-1")))
}

pub(super) fn fixture_with_credentials(
    replies: Vec<ScriptedReply>,
    key: Option<SecretString>,
) -> Fixture {
    build_fixture(replies, key, Arc::new(ProductionPayload))
}

pub(super) fn fixture_with_payload(
    replies: Vec<ScriptedReply>,
    payload: Arc<dyn RunPayload>,
) -> Fixture {
    build_fixture(replies, Some(secret("key-1")), payload)
}

fn build_fixture(
    replies: Vec<ScriptedReply>,
    key: Option<SecretString>,
    payload: Arc<dyn RunPayload>,
) -> Fixture {
    let transport = Arc::new(ScriptedTransport::new(replies));
    let dispatch = Arc::new(RecordingDispatch::default());
    let clock = TestClock::new();
    let sleeper = Arc::new(AdvancingSleeper::new(Arc::clone(&clock)));
    let tasks = TaskTracker::new();
    // Each port is cloned as its own type first, since Arc::clone cannot widen to the trait object the deps hold.
    let port_transport = Arc::clone(&transport);
    let port_clock = Arc::clone(&clock);
    let port_sleeper = Arc::clone(&sleeper);
    let billing = TinyfishRunBilling::new(
        RunSettings {
            api_base: "https://agent.tinyfish.ai".to_owned(),
            cost_per_step: decimal("0.016"),
        },
        BillingDeps {
            credentials: Arc::new(FixedCredentials { key }),
            transport: port_transport,
            monotonic: port_clock,
            sleeper: port_sleeper,
            clock: Arc::new(FixedSuccessClock),
            payload,
            tasks: BackgroundTasks {
                tasks: tasks.clone(),
            },
        },
    );
    Fixture {
        billing,
        transport,
        dispatch,
        sleeper,
        tasks,
    }
}

// LiteLLM's except around the payload build is only reachable through a build that fails.
pub(super) struct FailingPayload;

impl RunPayload for FailingPayload {
    fn build(&self, _input: PayloadInput<'_>) -> Result<HandlerOutcome, TinyfishError> {
        let source = serde_json::from_str::<Json>("not json").expect_err("the literal is not JSON");
        Err(TinyfishError::Encode { source })
    }
}

// The dispatch sink is injected per test so each call site passes the recorder it asserts on.
pub(super) fn dispatch_handle(dispatch: &Arc<RecordingDispatch>) -> Arc<RecordingDispatch> {
    Arc::clone(dispatch)
}
