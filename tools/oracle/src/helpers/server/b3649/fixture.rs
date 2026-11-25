use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::Redactor;
use betterlite_server::pass_through_logging::background_tasks::BackgroundTasks;
use betterlite_server::pass_through_logging::tinyfish::billing::{
    BillingDeps, ProductionPayload, RunSettings, TinyfishRunBilling,
};
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::json;
use tokio_util::task::TaskTracker;

use super::ports::{
    AdvancingSleeper, FixedCredentials, FixedSuccessClock, ScriptedTransport, TestClock,
};
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
    // Arc::clone cannot widen to the trait objects the deps hold, so each port is cloned as its own type first.
    let port_transport = Arc::clone(&transport);
    let port_clock = Arc::clone(&clock);
    let billing = TinyfishRunBilling::new(
        RunSettings {
            api_base: API_BASE.to_owned(),
            cost_per_step: Decimal::from_parts(16, 0, 0, false, 3),
        },
        BillingDeps {
            credentials: Arc::new(FixedCredentials {
                key: key.map(|text| SecretString::from(text.to_owned())),
            }),
            transport: port_transport,
            monotonic: port_clock,
            sleeper: Arc::new(AdvancingSleeper { clock }),
            clock: Arc::new(FixedSuccessClock { now: end_time }),
            payload: Arc::new(ProductionPayload),
            tasks: BackgroundTasks {
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
