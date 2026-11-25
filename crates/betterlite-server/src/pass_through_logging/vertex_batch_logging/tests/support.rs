#![cfg(test)]

use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use tokio::sync::mpsc;
use tokio::task::yield_now;
use tokio::time::timeout;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::pass_through_logging::anthropic_batch::{
    BatchLoggingMessage, BatchRegistration, ManagedBatchRegistry, RegistryError, ResponseIds,
    RouterModelIds,
};
use crate::pass_through_logging::vertex_batch_logging::{
    VertexBatchInput, VertexBatchLogged, VertexBatchLogging,
};

pub(crate) const RESPONSE_ID: &str = "resp-1";
pub(crate) const BATCH_ID: &str = "3814889423749775360";
pub(crate) const MODEL_PATH: &str = "publishers/google/models/gemini-2.5-flash";
pub(crate) const MODEL: &str = "gemini-2.5-flash";
pub(crate) const COLLECTION_ROUTE: &str =
    "/vertex_ai/v1/projects/p1/locations/us-central1/batchPredictionJobs";
pub(crate) const ID_SCOPED_ROUTE: &str =
    "/vertex_ai/v1/projects/p1/locations/us-central1/batchPredictionJobs/3814889423749775360";
pub(crate) const START: i64 = 1_767_225_600;

#[derive(Clone)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes).expect("UTF-8 log lines").to_owned()
    }

    pub(crate) async fn wait_for(&self, needle: &str) {
        let seen = async {
            while !self.text().contains(needle) {
                yield_now().await;
            }
        };
        timeout(Duration::from_secs(5), seen)
            .await
            .expect("the log line arrives");
    }
}

impl io::Write for Logs {
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

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(logs.clone())
        .finish();
    (logs, set_default(subscriber))
}

struct FixedIds;

impl ResponseIds for FixedIds {
    fn new_response_id(&self) -> String {
        RESPONSE_ID.to_owned()
    }
}

pub(crate) struct Router {
    ids: Vec<String>,
    pub(crate) asked: Mutex<Vec<String>>,
}

impl RouterModelIds for Router {
    fn model_ids(&self, model_name: &str) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_name.to_owned());
        self.ids.clone()
    }
}

pub(crate) fn router(ids: &[&str]) -> Arc<Router> {
    Arc::new(Router {
        ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        asked: Mutex::new(Vec::new()),
    })
}

struct Registry {
    sent: mpsc::Sender<BatchRegistration>,
    outcome: Result<(), RegistryError>,
}

impl ManagedBatchRegistry for Registry {
    fn store_unified_object_id(
        &self,
        request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>> {
        let sent = self.sent.clone();
        let outcome = self.outcome.clone();
        Box::pin(async move {
            let _delivered = sent.send(request).await;
            outcome
        })
    }
}

pub(crate) fn registry(
    outcome: Result<(), RegistryError>,
) -> (
    Arc<dyn ManagedBatchRegistry>,
    mpsc::Receiver<BatchRegistration>,
) {
    let (sent, received) = mpsc::channel(4);
    (Arc::new(Registry { sent, outcome }), received)
}

pub(crate) async fn next_registration(
    received: &mut mpsc::Receiver<BatchRegistration>,
) -> BatchRegistration {
    timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("a registration arrives")
        .expect("the registry records one")
}

pub(crate) fn logging(
    router_port: Option<Arc<dyn RouterModelIds>>,
    registry_port: Option<Arc<dyn ManagedBatchRegistry>>,
) -> VertexBatchLogging {
    VertexBatchLogging::new(router_port, registry_port, Arc::new(FixedIds))
}

pub(crate) struct Call {
    pub(crate) status_code: u16,
    pub(crate) body: String,
    pub(crate) url_route: &'static str,
    pub(crate) start_time: DateTime<Utc>,
    pub(crate) request_metadata: JsonObject,
}

impl Call {
    pub(crate) fn new(status_code: u16, body: &str, url_route: &'static str) -> Self {
        Self {
            status_code,
            body: body.to_owned(),
            url_route,
            start_time: DateTime::from_timestamp(START, 0).expect("a valid start"),
            request_metadata: JsonObject::new(),
        }
    }

    pub(crate) fn run(&self, handler: &VertexBatchLogging) -> (VertexBatchLogged, LoggingState) {
        let mut call_log = logging_state();
        let input = VertexBatchInput {
            status_code: self.status_code,
            body: self.body.as_bytes(),
            url_route: self.url_route,
            start_time: self.start_time,
            request_metadata: &self.request_metadata,
        };
        let logged = handler.batch_prediction_jobs_handler(input, &mut call_log);
        (logged, call_log)
    }
}

pub(crate) fn job_body(name: &str, model: Option<&str>) -> String {
    let model_field = model.map_or_else(String::new, |text| format!(r#","model":{text}"#));
    format!(
        r#"{{"name":"{name}","state":"JOB_STATE_PENDING","createTime":"2024-01-01T00:00:00.000000Z","inputConfig":{{"gcsSource":{{"uris":["gs://bucket/in.jsonl"]}}}}{model_field}}}"#
    )
}

pub(crate) fn success_body() -> String {
    job_body(
        &format!("projects/p1/locations/us-central1/batchPredictionJobs/{BATCH_ID}"),
        Some(&format!(r#""{MODEL_PATH}""#)),
    )
}

pub(crate) fn message(logged: &VertexBatchLogged) -> &BatchLoggingMessage {
    &logged.result.choices.first().expect("one choice").message
}

pub(crate) fn fields(logged: &VertexBatchLogged) -> Vec<(&str, &Json)> {
    message(logged)
        .provider_specific_fields
        .iter()
        .map(|(name, value)| (name.as_str(), value))
        .collect()
}

pub(crate) fn metadata(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

pub(crate) fn logging_state() -> LoggingState {
    let init = LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: DateTime::from_timestamp(START, 0)
            .expect("a valid start")
            .fixed_offset(),
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
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        &redactor,
        || "generated".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state")
}
