use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use uuid::Uuid;

use crate::health_check_entry::ports::{CatalogMode, CatalogModes, ProviderResolver};
use crate::health_check_entry::{
    HealthCheckEntry, HealthCheckFailure, HealthCheckOutcome, RawRequestRecord,
};
use crate::health_probe_modes::backend::{
    CheapestChatModels, HealthProbeBackend, OcrHealthDocuments,
};
use crate::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::logging::HealthProbeLogging;
use crate::health_probe_modes::params::HealthProbeParams;
use crate::health_probe_modes::response::HealthCheckResponse;

const NOW: &str = "2026-01-02T03:04:05+00:00";
const RAW_REQUEST_KEY: &str = "raw_request_typed_dict";

pub(super) enum Recorded {
    Mode {
        name: &'static str,
        parameters: IndexMap<String, Json>,
        logging: Option<Arc<dyn HealthProbeLogging>>,
    },
    Ocr {
        parameters: IndexMap<String, Json>,
        logging: Option<Arc<dyn HealthProbeLogging>>,
    },
    Realtime {
        model: String,
        provider: String,
        parameters: IndexMap<String, Json>,
        logging: Option<Arc<dyn HealthProbeLogging>>,
    },
}

impl Recorded {
    pub(super) fn name(&self) -> &'static str {
        match self {
            Self::Mode { name, .. } => name,
            Self::Ocr { .. } => "ocr",
            Self::Realtime { .. } => "realtime",
        }
    }

    pub(super) fn parameters(&self) -> &IndexMap<String, Json> {
        match self {
            Self::Mode { parameters, .. }
            | Self::Ocr { parameters, .. }
            | Self::Realtime { parameters, .. } => parameters,
        }
    }

    pub(super) fn logging(&self) -> Option<&Arc<dyn HealthProbeLogging>> {
        match self {
            Self::Mode { logging, .. }
            | Self::Ocr { logging, .. }
            | Self::Realtime { logging, .. } => logging.as_ref(),
        }
    }

    fn mode(
        name: &'static str,
        parameters: IndexMap<String, Json>,
        logging: Option<Arc<dyn HealthProbeLogging>>,
    ) -> Self {
        Self::Mode {
            name,
            parameters,
            logging,
        }
    }

    fn realtime(call: RealtimeCall) -> Self {
        Self::Realtime {
            model: call.model,
            provider: call.custom_llm_provider,
            parameters: call.model_params,
            logging: call.logging,
        }
    }
}

pub(super) struct FakeBackend {
    calls: Mutex<Vec<Recorded>>,
    headers: Mutex<BTreeMap<String, String>>,
    failure: Mutex<Option<String>>,
    raw_request: Mutex<Option<RawRequestRecord>>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            headers: Mutex::new(BTreeMap::new()),
            failure: Mutex::new(None),
            raw_request: Mutex::new(None),
        }
    }

    pub(super) fn reply(&self, headers: &[(&str, &str)]) {
        let mut reply = self.headers.lock().unwrap();
        reply.clear();
        for (name, value) in headers {
            reply.insert(String::from(*name), String::from(*value));
        }
    }

    pub(super) fn fail(&self, message: &str) {
        *self.failure.lock().unwrap() = Some(message.to_owned());
    }

    pub(super) fn record_raw_request(&self, record: RawRequestRecord) {
        *self.raw_request.lock().unwrap() = Some(record);
    }

    pub(super) fn take(&self) -> Recorded {
        self.calls.lock().unwrap().pop().expect("a recorded call")
    }

    pub(super) fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn note(&self, recorded: Recorded) -> Result<HealthProbeReply, HealthProbeError> {
        self.write_raw_request(recorded.logging());
        let failure = self.failure.lock().unwrap().clone();
        self.calls.lock().unwrap().push(recorded);
        match failure {
            Some(message) => Err(backend_failure(&message)),
            None => Ok(HealthProbeReply {
                hidden_headers: self.headers.lock().unwrap().clone(),
            }),
        }
    }

    fn write_raw_request(&self, logging: Option<&Arc<dyn HealthProbeLogging>>) {
        let configured = self.raw_request.lock().unwrap().clone();
        let Some(record) = configured else {
            return;
        };
        let Some(object) = logging else {
            return;
        };
        let Some(state) = object.logging_state() else {
            return;
        };
        let value = serde_json::to_value(&record).unwrap();
        let json: Json = serde_json::from_value(value).unwrap();
        state
            .lock()
            .unwrap()
            .model_call_details
            .insert(RAW_REQUEST_KEY.to_owned(), json);
    }
}

#[async_trait]
impl HealthProbeBackend for FakeBackend {
    async fn chat(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("chat", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn completion(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("completion", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn embedding(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("embedding", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn audio_speech(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("audio_speech", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn audio_transcription(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("audio_transcription", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn image_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("image_generation", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn image_edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("image_edit", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn video_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("video_generation", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("rerank", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn realtime(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note(Recorded::realtime(call))
    }

    async fn batch(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("batch", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn responses(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("responses", call.parameters, call.logging);
        self.note(recorded)
    }

    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note(Recorded::Ocr {
            parameters: call.parameters,
            logging: call.logging,
        })
    }

    async fn evaluation(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        let recorded = Recorded::mode("evaluation", IndexMap::new(), call.logging);
        self.note(recorded)
    }
}

pub(super) struct FakeCheapest {
    models: Mutex<Vec<String>>,
    request: Mutex<Option<(String, usize)>>,
}

impl FakeCheapest {
    fn new() -> Self {
        Self {
            models: Mutex::new(Vec::new()),
            request: Mutex::new(None),
        }
    }

    pub(super) fn set(&self, models: &[&str]) {
        let mut current = self.models.lock().unwrap();
        current.clear();
        for model in models {
            current.push(String::from(*model));
        }
    }

    pub(super) fn request(&self) -> Option<(String, usize)> {
        self.request.lock().unwrap().clone()
    }
}

impl CheapestChatModels for FakeCheapest {
    fn cheapest_chat_models(&self, custom_llm_provider: &str, limit: usize) -> Vec<String> {
        *self.request.lock().unwrap() = Some((custom_llm_provider.to_owned(), limit));
        self.models
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .collect()
    }
}

pub(super) struct FakeDocuments {
    missing: Mutex<bool>,
    seen: Mutex<Option<(String, String)>>,
}

impl FakeDocuments {
    fn new() -> Self {
        Self {
            missing: Mutex::new(false),
            seen: Mutex::new(None),
        }
    }

    pub(super) fn missing(&self) {
        *self.missing.lock().unwrap() = true;
    }

    pub(super) fn seen(&self) -> Option<(String, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl OcrHealthDocuments for FakeDocuments {
    fn health_check_document(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<OcrDocument, HealthProbeError> {
        *self.seen.lock().unwrap() = Some((model.to_owned(), custom_llm_provider.to_owned()));
        if *self.missing.lock().unwrap() {
            return Err(HealthProbeError::OcrDocumentUnavailable {
                model: model.to_owned(),
                custom_llm_provider: custom_llm_provider.to_owned(),
            });
        }
        Ok(OcrDocument::DocumentUrl {
            document_url: "data:application/pdf;base64,aGVhbHRo".to_owned(),
            extra: BTreeMap::new(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RequestedResolution {
    pub(super) model: String,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
}

pub(super) struct FakeResolver {
    resolved: Mutex<ResolvedModel>,
    failing: Mutex<bool>,
    request: Mutex<Option<RequestedResolution>>,
}

impl FakeResolver {
    fn new(model: &str, provider: &str) -> Self {
        Self {
            resolved: Mutex::new(resolved(model, provider)),
            failing: Mutex::new(false),
            request: Mutex::new(None),
        }
    }

    pub(super) fn resolve_to(&self, resolved: ResolvedModel) {
        *self.resolved.lock().unwrap() = resolved;
    }

    pub(super) fn failing(&self) {
        *self.failing.lock().unwrap() = true;
    }

    pub(super) fn request(&self) -> Option<RequestedResolution> {
        self.request.lock().unwrap().clone()
    }
}

impl ProviderResolver for FakeResolver {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError> {
        let requested = RequestedResolution {
            model: request.model.to_owned(),
            custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
            api_base: request.api_base.map(str::to_owned),
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
        };
        *self.request.lock().unwrap() = Some(requested);
        if *self.failing.lock().unwrap() {
            return Err(ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            });
        }
        Ok(self.resolved.lock().unwrap().clone())
    }
}

pub(super) struct FakeCatalog {
    modes: Mutex<BTreeMap<String, Option<String>>>,
}

impl FakeCatalog {
    fn new() -> Self {
        Self {
            modes: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn set(&self, model: &str, mode: Option<&str>) {
        self.modes
            .lock()
            .unwrap()
            .insert(model.to_owned(), mode.map(str::to_owned));
    }
}

impl CatalogModes for FakeCatalog {
    fn catalog_mode(&self, model: &str) -> CatalogMode {
        match self.modes.lock().unwrap().get(model) {
            None => CatalogMode::NotListed,
            Some(None) => CatalogMode::NoMode,
            Some(Some(mode)) => CatalogMode::Mode(mode.clone()),
        }
    }
}

pub(super) struct Harness {
    pub(super) entry: HealthCheckEntry,
    pub(super) backend: Arc<FakeBackend>,
    pub(super) cheapest: Arc<FakeCheapest>,
    pub(super) documents: Arc<FakeDocuments>,
    pub(super) resolver: Arc<FakeResolver>,
    pub(super) catalog: Arc<FakeCatalog>,
}

impl Harness {
    pub(super) fn new() -> Self {
        let backend = Arc::new(FakeBackend::new());
        let cheapest = Arc::new(FakeCheapest::new());
        let documents = Arc::new(FakeDocuments::new());
        let resolver = Arc::new(FakeResolver::new("resolved-model", "openai"));
        let catalog = Arc::new(FakeCatalog::new());
        let entry = HealthCheckEntry {
            backend: Arc::clone(&backend),
            cheapest: Arc::clone(&cheapest),
            documents: Arc::clone(&documents),
            resolver: Arc::clone(&resolver),
            catalog: Arc::clone(&catalog),
            redactor: Arc::new(Redactor::new(16).unwrap()),
            redact_secrets: true,
            clock: fixed_time,
            next_id: || Uuid::new_v4().to_string(),
        };
        Self {
            entry,
            backend,
            cheapest,
            documents,
            resolver,
            catalog,
        }
    }

    pub(super) async fn run(
        &self,
        params: HealthProbeParams,
        mode: Option<&str>,
    ) -> HealthCheckOutcome {
        self.entry
            .check_health(params, mode.map(str::to_owned), None, None)
            .await
    }
}

pub(super) fn params<T: Serialize>(entries: T) -> HealthProbeParams {
    let map = serde_json::from_value(serde_json::to_value(entries).unwrap()).unwrap();
    HealthProbeParams::from_map(map).unwrap()
}

pub(super) fn state_of(recorded: &Recorded) -> Arc<Mutex<LoggingState>> {
    recorded
        .logging()
        .expect("the call carries the logging object")
        .logging_state()
        .expect("the logging object exposes its state")
}

pub(super) fn healthy(outcome: HealthCheckOutcome) -> HealthCheckResponse {
    match outcome {
        HealthCheckOutcome::Healthy(response) => response,
        HealthCheckOutcome::Failed(failure) => panic!("the health check fails: {}", failure.error),
    }
}

pub(super) fn failed(outcome: HealthCheckOutcome) -> HealthCheckFailure {
    match outcome {
        HealthCheckOutcome::Healthy(_) => panic!("the health check succeeds"),
        HealthCheckOutcome::Failed(failure) => *failure,
    }
}

pub(super) fn stack_of(failure: &HealthCheckFailure) -> &str {
    failure
        .error
        .split_once("\nstack trace: ")
        .map_or("", |(_, stack)| stack)
}

pub(super) fn fixed_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(NOW).unwrap()
}

pub(super) fn assert_uuid4(value: &str) {
    assert_eq!(Uuid::parse_str(value).unwrap().get_version_num(), 4);
}

pub(super) fn resolved(model: &str, provider: &str) -> ResolvedModel {
    ResolvedModel {
        model: model.to_owned(),
        provider: ProviderIdentity::Custom(provider.to_owned()),
        api_base: None,
        api_key: None,
    }
}

fn backend_failure(message: &str) -> HealthProbeError {
    let context = ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    HealthProbeError::Backend {
        class: ErrorClass::Unclassified(Box::new(context)),
        message: message.to_owned(),
        source: Box::new(io::Error::other(message.to_owned())),
    }
}

pub(super) const HEALTHY_HEADERS: [(&str, &str); 3] = [
    ("x-ratelimit-remaining-requests", "9"),
    ("x-ratelimit-remaining-tokens", "8"),
    ("x-ms-region", "westeurope"),
];
