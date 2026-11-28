use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_vertex::vertex_relay_metadata::types::{
    CompletionCost, CompletionCostRequest, CustomLlmProvider,
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, StatusCode};
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::pass_through_logging::anthropic_batch::{
    BatchRegistration, ManagedBatchRegistry, RegistryError,
};
use crate::pass_through_logging::background_tasks::BackgroundTasks;
use crate::pass_through_logging::gemini_passthrough_logging::tests::support::{
    FakeCost, logging_object, timestamp,
};
use crate::pass_through_logging::vertex_batch_logging::VertexBatchLogging;
use crate::pass_through_logging::vertex_batch_logging::testing::{FixedResponseId, logging_state};
use crate::pass_through_logging::vertex_passthrough_handler::{
    VertexPassthroughHandler, VertexPassthroughInput,
};
use crate::pass_through_logging::vertex_predict_logging::VertexPredictLogger;

pub(crate) const METADATA_LISTS: [&str; 4] = [
    "vertex_ai_grounding_metadata",
    "vertex_ai_url_context_metadata",
    "vertex_ai_safety_results",
    "vertex_ai_citation_metadata",
];

pub(crate) struct Fixture {
    pub(crate) logging_obj: StreamLoggingObject,
    pub(crate) call_log: LoggingState,
    pub(crate) url_route: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
    pub(crate) request_body: Option<JsonMap>,
    pub(crate) kwargs: PassThroughLoggingKwargs,
}

pub(crate) fn fixture(url_route: &str, body: &str) -> Fixture {
    Fixture {
        logging_obj: logging_object(),
        call_log: logging_state().expect("a logging state"),
        url_route: url_route.to_owned(),
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
        request_body: None,
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

pub(crate) fn with_request_body(mut fixture: Fixture, body: &str) -> Fixture {
    let serde_json::Value::Object(fields) =
        serde_json::from_str(body).expect("a JSON object literal")
    else {
        panic!("the request body is an object");
    };
    fixture.request_body = Some(fields);
    fixture
}

pub(crate) fn input(fixture: &mut Fixture) -> VertexPassthroughInput<'_> {
    VertexPassthroughInput {
        logging_obj: &fixture.logging_obj,
        url_route: &fixture.url_route,
        result: "",
        start_time: timestamp(),
        end_time: timestamp(),
        cache_hit: false,
        status: StatusCode::OK,
        response_headers: &fixture.headers,
        response_body: &fixture.body,
        request_body: fixture.request_body.as_ref(),
        call_log: &mut fixture.call_log,
        kwargs: fixture.kwargs.clone(),
    }
}

pub(crate) fn items(value: &Json) -> &[Json] {
    match value {
        Json::List(values) => values,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            panic!("the value is a list")
        }
    }
}

// B3639 prices through B3638's trait, so the predict branches need their own recording adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PredictCall {
    pub(crate) model: String,
    pub(crate) provider: CustomLlmProvider,
    pub(crate) location: Option<String>,
    pub(crate) usage: Option<(i64, i64, i64)>,
}

pub(crate) struct PredictCost {
    price: Option<Decimal>,
    calls: Mutex<Vec<PredictCall>>,
}

impl PredictCost {
    fn new(price: Option<Decimal>) -> Arc<Self> {
        Arc::new(Self {
            price,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<PredictCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompletionCost for PredictCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Option<Decimal> {
        let usage = request.response.usage.as_ref().map(|usage| {
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens,
            )
        });
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PredictCall {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider,
                location: request.vertex_location.map(str::to_owned),
                usage,
            });
        self.price
    }
}

pub(crate) fn handler(cost: Arc<FakeCost>) -> (VertexPassthroughHandler, Arc<PredictCost>) {
    handler_with_predict(cost, Some(Decimal::ONE))
}

pub(crate) fn handler_with_predict(
    cost: Arc<FakeCost>,
    predict_price: Option<Decimal>,
) -> (VertexPassthroughHandler, Arc<PredictCost>) {
    let predict_cost = PredictCost::new(predict_price);
    let relay: Arc<dyn CompletionCost> = Arc::<PredictCost>::clone(&predict_cost);
    let predict = VertexPredictLogger::new(relay);
    let batch = VertexBatchLogging::new(
        None,
        None,
        Arc::new(FixedResponseId),
        BackgroundTasks::default(),
    );
    let passthrough = VertexPassthroughHandler::new(predict, batch, cost);
    (passthrough, predict_cost)
}

#[derive(Default)]
pub(crate) struct RecordingRegistry {
    registrations: Mutex<Vec<BatchRegistration>>,
}

impl RecordingRegistry {
    pub(crate) fn registrations(&self) -> Vec<BatchRegistration> {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ManagedBatchRegistry for RecordingRegistry {
    fn store_unified_object_id(
        &self,
        request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>> {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Box::pin(async { Ok(()) })
    }
}

pub(crate) fn handler_with_batch(
    cost: Arc<FakeCost>,
    registry: &Arc<RecordingRegistry>,
) -> VertexPassthroughHandler {
    let predict_cost = PredictCost::new(Some(Decimal::ONE));
    let relay: Arc<dyn CompletionCost> = predict_cost;
    let predict = VertexPredictLogger::new(relay);
    let shared: Arc<dyn ManagedBatchRegistry> = Arc::<RecordingRegistry>::clone(registry);
    let batch = VertexBatchLogging::new(
        None,
        Some(shared),
        Arc::new(FixedResponseId),
        BackgroundTasks::default(),
    );
    VertexPassthroughHandler::new(predict, batch, cost)
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured tracing output is UTF-8")
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

// tracing's default dispatcher is thread-scoped, so the guard keeps the captured events in this test.
pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
