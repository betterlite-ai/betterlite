use std::collections::VecDeque;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::Redactor;
use betterlite_server::pass_through_logging::tinyfish::billing::{
    BillingDeps, ProductionPayload, RunSettings, TinyfishRunBilling,
};
use betterlite_server::pass_through_logging::tinyfish::fetch::{
    RUN_API_KEY_HEADER, RunCredentials, RunFetchFuture, RunReply, RunRequest, RunTransport,
};
use betterlite_server::pass_through_logging::tinyfish::poll::{
    PollClock, PollSleepFuture, PollSleeper,
};
use betterlite_server::pass_through_logging::tinyfish::tasks::BillingTasks;
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::json;
use tokio_util::task::TaskTracker;

use super::view::Recorded;

// The oracle points the poller at no host, since every fetch is answered by the scripted transport.
pub(super) const API_BASE: &str = "https://agent.tinyfish.ai";
const START_TIME: &str = "2024-01-01T00:00:00+00:00";
const END_TIME: &str = "2024-01-01T00:00:05+00:00";

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

// The same shape the pass-through route builds: one request's logging state, owned by a dispatcher that does nothing.
pub(super) fn logging_object() -> Result<StreamLoggingObject, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_value(json!({
        "model": "fixture-model",
        "call_type": "pass_through_endpoint",
        "start_time": START_TIME,
        "litellm_call_id": "call-1",
        "function_id": "function-1",
        "stream": false,
    }))
    .or_value_error()?;
    let redactor = Redactor::new(16).or_value_error()?;
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        init,
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .or_value_error()?;
    Ok(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    })
}

// LiteLLM hands the poller's payload to the passthrough logging sink; the recorder is that sink for one case.
#[derive(Default)]
pub(super) struct Recorder {
    calls: Mutex<Vec<Recorded>>,
}

impl Recorder {
    // The poller bills once, so anything but one dispatch means the case saw a different path than expected.
    pub(super) fn only_call(&self) -> Result<Recorded, Box<HelperError>> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        match calls.as_slice() {
            [call] => Ok(call.clone()),
            other => Err(super::failure(&format!(
                "the billing sink dispatched {} times",
                other.len()
            ))),
        }
    }
}

#[async_trait]
impl StreamSuccessDispatch for Recorder {
    async fn dispatch_success(&self, call: StreamSuccessCall<'_>) {
        let response = match call.result.response() {
            Json::Object(entries) => super::view::text_entry(entries, "response"),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                cache_hit: Some(call.cache_hit),
                response,
            });
    }
}

#[derive(Clone)]
pub(super) struct ScriptedReply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub(super) fn replies(bodies: &[super::Reply]) -> Result<Vec<ScriptedReply>, Box<HelperError>> {
    bodies
        .iter()
        .map(|reply| {
            Ok(ScriptedReply {
                status: reply.status,
                body: serde_json::to_vec(&reply.body).or_value_error()?,
            })
        })
        .collect()
}

struct RecordedRequest {
    url: String,
    api_key: Option<String>,
}

pub(super) struct RequestLog {
    pub count: usize,
    pub paths: Vec<String>,
    pub api_keys: Vec<Option<String>>,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<ScriptedReply>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn next_reply(&self) -> Option<ScriptedReply> {
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        if replies.len() > 1 {
            replies.pop_front()
        } else {
            replies.front().cloned()
        }
    }

    // The same first and last one LiteLLM polls with: one scripted answer repeats until the loop ends.
    pub(super) fn request_log(&self) -> RequestLog {
        let requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        let mut paths: Vec<String> = Vec::new();
        let mut api_keys: Vec<Option<String>> = Vec::new();
        for request in requests.iter() {
            let path = match request.url.strip_prefix(API_BASE) {
                Some(rest) => rest.to_owned(),
                None => request.url.clone(),
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
            if !api_keys.contains(&request.api_key) {
                api_keys.push(request.api_key.clone());
            }
        }
        RequestLog {
            count: requests.len(),
            paths,
            api_keys,
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
            });
        let reply = self.next_reply();
        Box::pin(ready(match reply {
            Some(found) => Ok(RunReply {
                status: found.status,
                body: found.body,
            }),
            None => Err(TransportError::Rejected(HttpError::Timeout)),
        }))
    }
}

struct FixedCredentials {
    key: Option<SecretString>,
}

impl RunCredentials for FixedCredentials {
    fn api_key(&self) -> Option<SecretString> {
        self.key.clone()
    }
}

// LiteLLM reads time.monotonic for its absolute deadline, and the fake clock moves only when the poller sleeps.
#[derive(Default)]
pub(super) struct TestClock {
    elapsed: Mutex<Duration>,
}

impl PollClock for TestClock {
    fn now(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct AdvancingSleeper {
    clock: Arc<TestClock>,
}

impl PollSleeper for AdvancingSleeper {
    fn sleep(&self, duration: Duration) -> PollSleepFuture<'_> {
        let mut elapsed = self
            .clock
            .elapsed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *elapsed = elapsed.saturating_add(duration);
        Box::pin(ready(()))
    }
}

pub(super) struct FixedSuccessClock {
    pub now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedSuccessClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(super) struct Fixture {
    pub billing: TinyfishRunBilling,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub recorder: Arc<Recorder>,
    pub transport: Arc<ScriptedTransport>,
    pub start_time: DateTime<FixedOffset>,
}

pub(super) fn fixture(
    key: Option<&str>,
    replies: Vec<ScriptedReply>,
) -> Result<Fixture, Box<HelperError>> {
    let logging_obj = Arc::new(logging_object()?);
    let recorder = Arc::new(Recorder::default());
    let transport = Arc::new(ScriptedTransport::new(replies));
    let clock = Arc::new(TestClock::default());
    let start_time = DateTime::parse_from_rfc3339(START_TIME).or_value_error()?;
    let end_time = DateTime::parse_from_rfc3339(END_TIME).or_value_error()?;
    let billing = TinyfishRunBilling::new(
        RunSettings {
            api_base: API_BASE.to_owned(),
            cost_per_step: Decimal::from_parts(16, 0, 0, false, 3),
        },
        BillingDeps {
            credentials: Arc::new(FixedCredentials {
                key: key.map(|text| SecretString::from(text.to_owned())),
            }),
            transport: Arc::clone(&transport),
            monotonic: Arc::clone(&clock),
            sleeper: Arc::new(AdvancingSleeper { clock }),
            clock: Arc::new(FixedSuccessClock { now: end_time }),
            payload: Arc::new(ProductionPayload),
            tasks: BillingTasks {
                tasks: TaskTracker::new(),
            },
        },
    );
    Ok(Fixture {
        billing,
        logging_obj,
        recorder,
        transport,
        start_time,
    })
}
