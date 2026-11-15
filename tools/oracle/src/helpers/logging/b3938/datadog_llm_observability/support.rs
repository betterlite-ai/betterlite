use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::datadog::apm::{Span, Tracer};
use betterlite_logging::datadog_llm_metrics::token_counting::ToolTokenCounter;
use betterlite_logging::datadog_llm_observability::error::DatadogLlmObsError;
use betterlite_logging::datadog_llm_observability::logger::{
    DatadogLlmObsDeps, DatadogLlmObsLogger,
};
use betterlite_logging::datadog_llm_observability::settings::{
    DatadogLlmObsInit, DatadogLlmObsInitParams,
};
use betterlite_logging::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{Map, Value, json};

use super::{ActionCall, Reply, Scenario};
use crate::helpers::HelperError;
use crate::helpers::OrHelperError as _;

pub(super) type Logger = DatadogLlmObsLogger<TestCounter>;

pub(super) struct TestCounter;

impl ToolTokenCounter for TestCounter {
    type Failure = Infallible;

    fn count_text(
        &self,
        _model: &str,
        _text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Ok(0))
    }
}

struct TraceSpan {
    trace_id: Option<u128>,
}

impl Span for TraceSpan {
    fn trace_id(&self) -> Option<u128> {
        self.trace_id
    }
}

impl AsRef<dyn Span> for TraceSpan {
    fn as_ref(&self) -> &(dyn Span + 'static) {
        self
    }
}

struct FakeTracer {
    span: Option<TraceSpan>,
}

impl Tracer for FakeTracer {
    fn current_span(&self) -> Option<&dyn Span> {
        self.span.as_ref().map(AsRef::as_ref)
    }
}

#[derive(Debug, Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<Vec<Reply>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn call_record(request: &PostRequest) -> Value {
    let mut headers = Map::new();
    for (name, value) in &request.headers {
        headers.insert(
            name.as_str().to_ascii_lowercase(),
            Value::String(value.to_str().unwrap_or_default().to_owned()),
        );
    }
    json!({
        "url": request.url,
        "headers": headers,
        "body": serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null),
    })
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call_record(&request));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .ok_or(TransportFailure::Request(
                betterlite_http::proxy::transport_error::TransportError::Rejected(
                    betterlite_http::error::HttpError::Timeout,
                ),
            ))?;
        Ok(TransportResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}

fn init(scenario: &Scenario) -> DatadogLlmObsInit {
    DatadogLlmObsInit {
        dd_api_key: scenario.init.dd_api_key.clone().map(SecretString::from),
        dd_site: scenario.init.dd_site.clone(),
        dd_agent_host: scenario.init.dd_agent_host.clone(),
        dd_agent_port: scenario.init.dd_agent_port.clone(),
        turn_off_message_logging: scenario.init.turn_off_message_logging,
        message_logging: scenario.init.message_logging,
        batch_size: scenario.init.batch_size,
        flush_interval_seconds: scenario.init.flush_interval,
        params: scenario.params.map(|params| DatadogLlmObsInitParams {
            turn_off_message_logging: params.turn_off_message_logging,
        }),
        global_turn_off: scenario.init.global_turn_off.unwrap_or(false),
    }
}

pub(super) fn event(call: &ActionCall) -> Result<Arc<LoggerEvent>, Box<HelperError>> {
    let metadata = call.metadata.clone().unwrap_or(Json::Null);
    let mut payload = call.payload.as_ref().clone();
    payload.start_time = Decimal::try_from(call.start_time).or_value_error()?;
    payload.end_time = Decimal::try_from(call.end_time).or_value_error()?;
    let stream = payload.stream == Some(true);
    Ok(Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(metadata),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream,
        guardrail_snapshot: None,
    }))
}

pub(super) fn generated_id() -> String {
    // stable so both sides of the oracle compare the same invented span id
    "generated-id".to_owned()
}

pub(super) fn logger_for(
    scenario: &Scenario,
    transport: &Arc<ScriptedTransport>,
) -> Result<Arc<Logger>, DatadogLlmObsError> {
    let span = scenario.apm_trace_id.map(|trace_id| TraceSpan {
        trace_id: Some(trace_id),
    });
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(FakeTracer { span });
    let shared = Arc::clone(transport);
    let concrete: Arc<dyn Transport> = shared;
    DatadogLlmObsLogger::new(
        &init(scenario),
        &scenario.environment,
        Arc::new(TestCounter),
        DatadogLlmObsDeps {
            transport: concrete,
            tracer,
            ids: generated_id,
            tasks: None,
        },
    )
}
