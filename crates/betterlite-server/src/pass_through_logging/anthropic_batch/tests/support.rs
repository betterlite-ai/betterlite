#![cfg(test)]

use std::io;
use std::str::from_utf8;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::batches::config::{AnthropicBatchesConfig, BatchesConfig as _};
use betterlite_anthropic::batches::timestamps::Clock;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_auth::user_roles::UserRole;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::batches::records::BatchObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, Offset as _, Utc};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use tokio::sync::mpsc;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::pass_through_logging::anthropic_batch::{
    AnthropicBatchLogging, BatchCreationInput, BatchCreationLogged, BatchLoggingMessage,
    BatchRegistrant, BatchRegistration, ManagedBatchRegistry, RegistryError, ResponseIds,
    RouterModelIds,
};
use crate::pass_through_logging::background_tasks::BackgroundTasks;

pub(crate) const BATCH_ID: &str = "msgbatch_1";
pub(crate) const MODEL: &str = "claude-x";
pub(crate) const RESPONSE_ID: &str = "resp-1";
pub(crate) const ID_SCOPED_ROUTE: &str = "/v1/messages/batches/msgbatch_1";
pub(crate) const COLLECTION_ROUTE: &str = "/v1/messages/batches";
const START: &str = "2026-01-01T00:00:00Z";

#[derive(Clone)]
pub(crate) struct LogCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
    line: mpsc::Sender<()>,
    needle: &'static str,
}

impl LogCapture {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.bytes.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }

    fn saw_needle(&self, bytes: &[u8]) -> bool {
        !self.needle.is_empty()
            && bytes
                .windows(self.needle.len())
                .any(|window| window == self.needle.as_bytes())
    }
}

impl io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes = self.bytes.lock().unwrap_or_else(PoisonError::into_inner);
        bytes.extend_from_slice(buf);
        let found = self.saw_needle(&bytes);
        drop(bytes);
        if found {
            let _sent = self.line.try_send(());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogCapture {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        Self {
            bytes: Arc::clone(&self.bytes),
            line: self.line.clone(),
            needle: self.needle,
        }
    }
}

pub(crate) fn capture_logs(needle: &'static str) -> (LogCapture, mpsc::Receiver<()>, DefaultGuard) {
    let (line, lines) = mpsc::channel(4);
    let capture = LogCapture {
        bytes: Arc::new(Mutex::new(Vec::new())),
        line,
        needle,
    };
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(capture.clone())
        .finish();
    (capture, lines, set_default(subscriber))
}

pub(crate) struct Creation {
    status_code: u16,
    body: Vec<u8>,
    url_route: String,
    start: DateTime<Utc>,
    request_body: Option<Json>,
    request_metadata: JsonObject,
}

impl Creation {
    pub(crate) fn new(status_code: u16, body: Vec<u8>, url_route: &str) -> Self {
        Self {
            status_code,
            body,
            url_route: url_route.to_owned(),
            start: start_time(),
            request_body: None,
            request_metadata: JsonObject::new(),
        }
    }

    pub(crate) fn success(model: &str) -> Self {
        let request_body = format!(r#"{{"model":"{model}"}}"#);
        Self::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE).with_request_body(&request_body)
    }

    pub(crate) fn with_request_body(mut self, text: &str) -> Self {
        self.request_body = Some(json(text));
        self
    }

    pub(crate) fn with_metadata(mut self, text: &str) -> Self {
        self.request_metadata = metadata(text);
        self
    }

    pub(crate) fn with_start_time(mut self, start: DateTime<Utc>) -> Self {
        self.start = start;
        self
    }

    pub(crate) fn input(&self) -> BatchCreationInput<'_> {
        BatchCreationInput {
            status_code: self.status_code,
            body: &self.body,
            url_route: &self.url_route,
            start_time: self.start,
            request_body: self.request_body.as_ref(),
            request_metadata: &self.request_metadata,
        }
    }
}

pub(crate) struct FixedIds(pub(crate) &'static str);

impl ResponseIds for FixedIds {
    fn new_response_id(&self) -> String {
        self.0.to_owned()
    }
}

pub(crate) struct CountingIds {
    next: AtomicU32,
}

impl CountingIds {
    pub(crate) fn new() -> Self {
        Self {
            next: AtomicU32::new(1),
        }
    }
}

impl ResponseIds for CountingIds {
    fn new_response_id(&self) -> String {
        let value = self.next.fetch_add(1, Ordering::SeqCst);
        format!("id-{value}")
    }
}

pub(crate) struct RecordingRouter {
    ids: Vec<String>,
    asked: Mutex<Vec<String>>,
}

impl RecordingRouter {
    pub(crate) fn new(ids: &[&str]) -> Self {
        Self {
            ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn asked(&self) -> Vec<String> {
        let asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        asked.clone()
    }
}

impl RouterModelIds for RecordingRouter {
    fn model_ids(&self, model_name: &str) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_name.to_owned());
        self.ids.clone()
    }
}

pub(crate) fn router(ids: &[&str]) -> Arc<RecordingRouter> {
    Arc::new(RecordingRouter::new(ids))
}

pub(crate) fn router_port(ids: &[&str]) -> Arc<dyn RouterModelIds> {
    Arc::new(RecordingRouter::new(ids))
}

pub(crate) fn router_port_of(recording: &Arc<RecordingRouter>) -> Arc<dyn RouterModelIds> {
    Arc::<RecordingRouter>::clone(recording)
}

pub(crate) struct RecordingRegistry {
    registrations: mpsc::Sender<BatchRegistration>,
    outcome: Result<(), RegistryError>,
}

impl ManagedBatchRegistry for RecordingRegistry {
    fn store_unified_object_id(
        &self,
        request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>> {
        let registrations = self.registrations.clone();
        let outcome = self.outcome.clone();
        Box::pin(async move {
            let _sent = registrations.send(request).await;
            outcome
        })
    }
}

pub(crate) fn registry(
    outcome: Result<(), RegistryError>,
) -> (Arc<RecordingRegistry>, mpsc::Receiver<BatchRegistration>) {
    let (registrations, received) = mpsc::channel(4);
    (
        Arc::new(RecordingRegistry {
            registrations,
            outcome,
        }),
        received,
    )
}

pub(crate) fn registry_port(
    outcome: Result<(), RegistryError>,
) -> (
    Option<Arc<dyn ManagedBatchRegistry>>,
    mpsc::Receiver<BatchRegistration>,
) {
    let (recording, received) = registry(outcome);
    let port: Option<Arc<dyn ManagedBatchRegistry>> = Some(recording);
    (port, received)
}

pub(crate) fn assert_registrant_fields(registrant: &BatchRegistrant) {
    assert_eq!(registrant.user_id.as_deref(), Some("user-1"));
    assert_eq!(registrant.team_id.as_deref(), Some("team-1"));
    let api_key = registrant.api_key.as_ref().expect("the key is stored");
    assert_eq!(api_key.expose_secret(), "sk-key");
    let scalars = (
        registrant.team_alias.as_deref(),
        registrant.user_role,
        registrant.user_email.as_deref(),
        registrant.max_budget,
        registrant.spend,
        registrant.tpm_limit,
        registrant.rpm_limit,
        registrant.budget_duration.as_deref(),
    );
    let expected = (
        None,
        UserRole::Customer,
        None,
        None,
        Decimal::ZERO,
        None,
        None,
        None,
    );
    assert_eq!(scalars, expected);
    let absent = (
        registrant.budget_reset_at,
        registrant.max_parallel_requests,
        registrant.allowed_model_region.as_deref(),
        registrant.key_alias.as_deref(),
        registrant.models.len(),
        registrant.metadata.len(),
        registrant.permissions.len(),
        registrant.model_max_budget.len(),
        registrant.model_spend.len(),
    );
    assert_eq!(absent, (None, None, None, None, 0, 0, 0, 0, 0));
}

pub(crate) fn handler(
    router_ids: Option<Arc<dyn RouterModelIds>>,
    registry_hook: Option<Arc<dyn ManagedBatchRegistry>>,
) -> AnthropicBatchLogging {
    handler_with_ids(router_ids, registry_hook, Arc::new(FixedIds(RESPONSE_ID)))
}

pub(crate) fn handler_with_ids(
    router_port: Option<Arc<dyn RouterModelIds>>,
    registry_port: Option<Arc<dyn ManagedBatchRegistry>>,
    ids: Arc<dyn ResponseIds>,
) -> AnthropicBatchLogging {
    AnthropicBatchLogging::new(router_port, registry_port, ids, BackgroundTasks::default())
}

pub(crate) fn run(
    logging: &AnthropicBatchLogging,
    creation: &Creation,
) -> (BatchCreationLogged, LoggingState) {
    let mut call_log = logging_state();
    let logged = logging.batch_creation_handler(creation.input(), &mut call_log);
    (logged, call_log)
}

pub(crate) fn message(logged: &BatchCreationLogged) -> &BatchLoggingMessage {
    let choice = logged
        .result
        .choices
        .first()
        .expect("the record has one choice");
    &choice.message
}

pub(crate) fn provider_fields(logged: &BatchCreationLogged) -> &IndexMap<String, Json> {
    &message(logged).provider_specific_fields
}

pub(crate) fn field<'fields>(fields: &'fields IndexMap<String, Json>, name: &str) -> &'fields Json {
    fields.get(name).expect("the fields carry the key")
}

pub(crate) fn text<'fields>(fields: &'fields IndexMap<String, Json>, name: &str) -> &'fields str {
    match field(fields, name) {
        Json::Text(value) => value,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the field is text")
        }
    }
}

pub(crate) fn start_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(START)
        .expect("a valid timestamp")
        .with_timezone(&Utc)
}

pub(crate) fn batch_body(id: &str) -> Vec<u8> {
    let body = format!(
        r#"{{"id":"{id}","type":"message_batch","processing_status":"in_progress","request_counts":{{"processing":0,"succeeded":0,"errored":0,"canceled":0,"expired":0}},"created_at":"2026-01-01T00:00:00Z","expires_at":"2026-01-02T00:00:00Z","ended_at":null}}"#
    );
    body.into_bytes()
}

pub(crate) fn batch_object() -> BatchObject {
    let config = AnthropicBatchesConfig {
        settings: AnthropicSettings::default(),
    };
    let clock = Clock {
        now: start_time().timestamp(),
        zone: Utc.fix(),
    };
    config
        .transform_retrieve_batch_response(&batch_body(BATCH_ID), &clock)
        .expect("the fixture batch decodes")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON value")
}

pub(crate) fn metadata(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

pub(crate) fn logging_state() -> LoggingState {
    let init = LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: start_time().fixed_offset(),
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
