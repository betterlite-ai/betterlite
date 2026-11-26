use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::ResponseDetails;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, Messages, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_ocr::ocr_auth::{
    AwsRequestSigner, AwsSigningRequest, AzureTokenSource, VertexAccessToken, VertexTokenSource,
};
use betterlite_ocr::ocr_config::PreparedOcrRequest;
use betterlite_ocr::ocr_config::connection::OcrSourced;
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_ocr::ocr_error::{OcrError, OcrSecretError};
use betterlite_ocr::ocr_http::{OcrClient, SecretSource};
use betterlite_ocr::ocr_settings::OcrSettings;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::correlation::{CallOutcome, ChatStream};
use betterlite_sdk::call_wrapper::error::{CallError, plain_error};
use betterlite_sdk::call_wrapper::state::LoggingHandle;
use betterlite_sdk::call_wrapper::target::CallTarget;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::timeout;
use uuid::Uuid;

use crate::health_check_entry::HealthCheckEntry;
use crate::health_check_entry::ports::{CatalogMode, CatalogModes, ProviderResolver};
use crate::health_native_backend::backend::NativeHealthBackend;
use crate::health_native_backend::evaluation::EvaluationSetup;
use crate::health_native_backend::seams::{
    BatchProbeCall, EmbeddingProbeCall, EvaluationProbeCall, ImageEditProbeCall,
    ImageGenerationProbeCall, OcrProbeCall, RealtimeProbeCall, RerankProbeCall, ResponsesProbeCall,
    SpeechProbeCall, TextCompletionProbeCall, TranscriptionProbeCall, VideoGenerationProbeCall,
};
use crate::health_native_backend::wiring::{HealthEntryDeps, build_health_check_entry};
use crate::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::logging::ProbeLogging;
use crate::health_probe_modes::params::HealthProbeParams;

const NOW: &str = "2026-01-02T03:04:05+00:00";

pub(super) const HEALTHY_HEADERS: [(&str, &str); 3] = [
    ("x-ratelimit-remaining-requests", "9"),
    ("x-ratelimit-remaining-tokens", "8"),
    ("x-ms-region", "westeurope"),
];

pub(super) struct ChatCall {
    pub(super) request: RequestMap,
    pub(super) logging: Option<LoggingHandle>,
}

pub(super) struct FakeChat {
    response: Mutex<CallOutcome>,
    failure: Mutex<Option<CallError>>,
    calls: Mutex<Vec<ChatCall>>,
    retries: AtomicUsize,
}

impl FakeChat {
    fn new() -> Self {
        Self {
            response: Mutex::new(CallOutcome::Response(Json::Null)),
            failure: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
            retries: AtomicUsize::new(0),
        }
    }

    pub(super) fn answer(&self, headers: &[(&str, &str)]) {
        *self.response.lock().unwrap() = CallOutcome::Response(chat_body(headers));
    }

    pub(super) fn stream(&self) {
        *self.response.lock().unwrap() = CallOutcome::ChatStream(ChatStream {
            model_response: Json::Null,
            json_mode: false,
            converted_from_mock: false,
        });
    }

    pub(super) fn fail(&self, message: &str) {
        let class = ErrorClass::Authentication(Box::new(ResponseDetails::for_provider(
            String::from(message),
            None,
            "openai",
        )));
        *self.failure.lock().unwrap() = Some(CallError::from(class));
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub(super) fn retries(&self) -> usize {
        self.retries.load(Ordering::SeqCst)
    }

    pub(super) fn take(&self) -> ChatCall {
        self.calls
            .lock()
            .unwrap()
            .pop()
            .expect("a recorded chat call")
    }

    fn record(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ChatCall {
                request,
                logging: logging.cloned(),
            });
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(error) = failure else {
            return Ok(self
                .response
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone());
        };
        Err(error)
    }
}

#[async_trait]
impl CallTarget for FakeChat {
    fn name(&self) -> &'static str {
        "completion"
    }

    async fn call(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        self.record(request, logging)
    }

    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        let _retried = self.retries.fetch_add(1, Ordering::SeqCst);
        self.record(request, logging)
    }
}

pub(super) enum OwnerCall {
    Mode {
        mode: &'static str,
        parameters: IndexMap<String, Json>,
        api_key: Option<String>,
        logging: bool,
    },
    Transcription {
        parameters: IndexMap<String, Json>,
        file: Vec<u8>,
        api_key: Option<String>,
        logging: bool,
    },
    ImageEdit {
        parameters: IndexMap<String, Json>,
        image: Vec<u8>,
        prompt: &'static str,
        api_key: Option<String>,
        logging: bool,
    },
    Ocr {
        parameters: IndexMap<String, Json>,
        document: OcrDocument,
        api_key: Option<String>,
        logging: bool,
    },
    Realtime {
        model: String,
        custom_llm_provider: String,
        api_base: Option<String>,
        api_version: Option<String>,
        parameters: IndexMap<String, Json>,
        api_key: Option<String>,
        logging: bool,
    },
}

impl OwnerCall {
    fn mode(mode: &'static str, call: ProbeCall) -> Self {
        Self::Mode {
            mode,
            parameters: call.parameters,
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            logging: call.logging.is_some(),
        }
    }

    fn transcription(call: AudioTranscriptionCall) -> Self {
        Self::Transcription {
            parameters: call.parameters,
            file: call.file,
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            logging: call.logging.is_some(),
        }
    }

    fn image_edit(call: ImageEditCall) -> Self {
        Self::ImageEdit {
            parameters: call.parameters,
            image: call.image,
            prompt: call.prompt,
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            logging: call.logging.is_some(),
        }
    }

    fn ocr(call: OcrCall) -> Self {
        Self::Ocr {
            parameters: call.parameters,
            document: call.document,
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            logging: call.logging.is_some(),
        }
    }

    fn realtime(call: RealtimeCall) -> Self {
        Self::Realtime {
            model: call.model,
            custom_llm_provider: call.custom_llm_provider,
            api_base: call.api_base,
            api_version: call.api_version,
            parameters: call.model_params,
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            logging: call.logging.is_some(),
        }
    }

    pub(super) fn mode_name(&self) -> &'static str {
        match self {
            Self::Mode { mode, .. } => mode,
            Self::Transcription { .. } => "audio_transcription",
            Self::ImageEdit { .. } => "image_edit",
            Self::Ocr { .. } => "ocr",
            Self::Realtime { .. } => "realtime",
        }
    }

    pub(super) fn parameters(&self) -> &IndexMap<String, Json> {
        match self {
            Self::Mode { parameters, .. }
            | Self::Transcription { parameters, .. }
            | Self::ImageEdit { parameters, .. }
            | Self::Ocr { parameters, .. }
            | Self::Realtime { parameters, .. } => parameters,
        }
    }
}

pub(super) struct FakeOwner {
    calls: Mutex<Vec<OwnerCall>>,
    headers: Mutex<BTreeMap<String, String>>,
    failure: Mutex<Option<String>>,
}

impl FakeOwner {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            headers: Mutex::new(BTreeMap::new()),
            failure: Mutex::new(None),
        }
    }

    pub(super) fn reply(&self, headers: &[(&str, &str)]) {
        *self.headers.lock().unwrap() = headers_map(headers);
    }

    pub(super) fn fail(&self, message: &str) {
        *self.failure.lock().unwrap() = Some(String::from(message));
    }

    pub(super) fn take(&self) -> OwnerCall {
        self.calls
            .lock()
            .unwrap()
            .pop()
            .expect("a recorded owner call")
    }

    pub(super) fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn finish(&self, call: OwnerCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(message) = failure else {
            return Ok(HealthProbeReply {
                hidden_headers: self
                    .headers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
            });
        };
        Err(owner_failure(message))
    }
}

#[async_trait]
impl TextCompletionProbeCall for FakeOwner {
    async fn complete(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("completion", call))
    }
}

#[async_trait]
impl EmbeddingProbeCall for FakeOwner {
    async fn embed(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("embedding", call))
    }
}

#[async_trait]
impl SpeechProbeCall for FakeOwner {
    async fn speak(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("audio_speech", call))
    }
}

#[async_trait]
impl TranscriptionProbeCall for FakeOwner {
    async fn transcribe(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::transcription(call))
    }
}

#[async_trait]
impl ImageGenerationProbeCall for FakeOwner {
    async fn generate(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("image_generation", call))
    }
}

#[async_trait]
impl ImageEditProbeCall for FakeOwner {
    async fn edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::image_edit(call))
    }
}

#[async_trait]
impl RerankProbeCall for FakeOwner {
    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("rerank", call))
    }
}

#[async_trait]
impl BatchProbeCall for FakeOwner {
    async fn list_batches(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("batch", call))
    }
}

#[async_trait]
impl ResponsesProbeCall for FakeOwner {
    async fn create(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("responses", call))
    }
}

#[async_trait]
impl OcrProbeCall for FakeOwner {
    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::ocr(call))
    }
}

#[async_trait]
impl VideoGenerationProbeCall for FakeOwner {
    async fn generate(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::mode("video_generation", call))
    }
}

#[async_trait]
impl RealtimeProbeCall for FakeOwner {
    async fn probe(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.finish(OwnerCall::realtime(call))
    }
}

pub(super) struct FakeEvaluation {
    calls: Mutex<Vec<EvaluationCall>>,
    headers: Mutex<BTreeMap<String, String>>,
    failure: Mutex<Option<String>>,
}

impl FakeEvaluation {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            headers: Mutex::new(BTreeMap::new()),
            failure: Mutex::new(None),
        }
    }

    pub(super) fn reply(&self, headers: &[(&str, &str)]) {
        *self.headers.lock().unwrap() = headers_map(headers);
    }

    pub(super) fn fail(&self, message: &str) {
        *self.failure.lock().unwrap() = Some(String::from(message));
    }

    pub(super) fn take(&self) -> EvaluationCall {
        self.calls
            .lock()
            .unwrap()
            .pop()
            .expect("a recorded evaluation call")
    }

    pub(super) fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl EvaluationProbeCall for FakeEvaluation {
    async fn evaluate(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(message) = failure {
            return Err(owner_failure(message));
        }
        Ok(HealthProbeReply {
            hidden_headers: self
                .headers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        })
    }
}

#[derive(Default)]
pub(crate) struct EvalSetup {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    variables: BTreeMap<String, String>,
}

impl EvaluationSetup for EvalSetup {
    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.variables,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn globals(&self) -> LiteLlmGlobals {
        LiteLlmGlobals::default()
    }

    fn azure_ai(&self) -> AzureAiConfig {
        AzureAiConfig {
            complete_url: Box::new(|base: &str, _model: &str| base.to_owned()),
        }
    }

    fn global_drop_params(&self) -> bool {
        false
    }
}

pub(super) struct FakePrices {
    entries: Mutex<BTreeMap<(String, String), PriceRecord>>,
    provider: Mutex<Option<String>>,
}

impl FakePrices {
    fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            provider: Mutex::new(None),
        }
    }

    pub(super) fn chat(&self, provider: &str, model: &str, input: &str, output: &str) {
        let mut record = PriceRecord::new();
        record.insert(String::from("mode"), Scalar::Text(String::from("chat")));
        record.insert(String::from("input_cost_per_token"), price(input));
        record.insert(String::from("output_cost_per_token"), price(output));
        self.store(provider, model, record);
    }

    pub(super) fn embedding(&self, provider: &str, model: &str) {
        let mut record = PriceRecord::new();
        record.insert(
            String::from("mode"),
            Scalar::Text(String::from("embedding")),
        );
        record.insert(String::from("input_cost_per_token"), price("0"));
        record.insert(String::from("output_cost_per_token"), price("0"));
        self.store(provider, model, record);
    }

    pub(super) fn provider(&self) -> Option<String> {
        self.provider.lock().unwrap().clone()
    }

    fn store(&self, provider: &str, model: &str, record: PriceRecord) {
        let key = (String::from(provider), String::from(model));
        self.entries.lock().unwrap().insert(key, record);
    }
}

impl PriceLookup for FakePrices {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let provider = custom_llm_provider?;
        *self.provider.lock().unwrap() = Some(String::from(provider));
        let key = (String::from(provider), String::from(model));
        self.entries.lock().unwrap().get(&key).cloned()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

pub(super) struct FakeResolver {
    pub(super) resolved: ResolvedModel,
}

impl ProviderResolver for FakeResolver {
    fn resolve(&self, _request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError> {
        Ok(self.resolved.clone())
    }
}

pub(super) struct FakeCatalog;

impl CatalogModes for FakeCatalog {
    fn catalog_mode(&self, _model: &str) -> CatalogMode {
        CatalogMode::NotListed
    }
}

pub(super) struct Harness {
    pub(super) chat: Arc<FakeChat>,
    pub(super) owner: Arc<FakeOwner>,
    pub(super) evaluation: Arc<FakeEvaluation>,
    pub(super) prices: Arc<FakePrices>,
    model_sets: Arc<ProviderModelSets>,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self::with_models(&[])
    }

    pub(super) fn with_models(models: &[(&str, &[&str])]) -> Self {
        let mut model_sets = ProviderModelSets::default();
        for (provider, names) in models {
            let set: BTreeSet<String> = names.iter().map(|name| String::from(*name)).collect();
            model_sets
                .models_by_provider
                .insert(String::from(*provider), set);
        }
        Self {
            chat: Arc::new(FakeChat::new()),
            owner: Arc::new(FakeOwner::new()),
            evaluation: Arc::new(FakeEvaluation::new()),
            prices: Arc::new(FakePrices::new()),
            model_sets: Arc::new(model_sets),
        }
    }

    pub(super) fn backend(&self) -> NativeHealthBackend {
        let chat: Arc<dyn CallTarget> = Arc::<FakeChat>::clone(&self.chat);
        let completion: Arc<dyn TextCompletionProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let embedding: Arc<dyn EmbeddingProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let audio_speech: Arc<dyn SpeechProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let audio_transcription: Arc<dyn TranscriptionProbeCall> =
            Arc::<FakeOwner>::clone(&self.owner);
        let image_generation: Arc<dyn ImageGenerationProbeCall> =
            Arc::<FakeOwner>::clone(&self.owner);
        let image_edit: Arc<dyn ImageEditProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let rerank: Arc<dyn RerankProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let batch: Arc<dyn BatchProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let responses: Arc<dyn ResponsesProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let ocr: Arc<dyn OcrProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let video_generation: Arc<dyn VideoGenerationProbeCall> =
            Arc::<FakeOwner>::clone(&self.owner);
        let realtime_probe: Arc<dyn RealtimeProbeCall> = Arc::<FakeOwner>::clone(&self.owner);
        let evaluation: Arc<dyn EvaluationProbeCall> =
            Arc::<FakeEvaluation>::clone(&self.evaluation);
        let prices: Arc<dyn PriceLookup + Send + Sync> = Arc::<FakePrices>::clone(&self.prices);
        NativeHealthBackend {
            chat,
            completion,
            embedding,
            audio_speech,
            audio_transcription,
            image_generation,
            image_edit,
            rerank,
            batch,
            responses,
            ocr,
            video_generation,
            realtime_probe,
            evaluation,
            prices,
            model_sets: Arc::clone(&self.model_sets),
        }
    }

    pub(super) fn entry(&self, model: &str, provider: &str) -> HealthCheckEntry {
        build_health_check_entry(HealthEntryDeps {
            backend: self.backend(),
            resolver: Arc::new(FakeResolver {
                resolved: ResolvedModel {
                    model: String::from(model),
                    provider: ProviderIdentity::Custom(String::from(provider)),
                    api_base: None,
                    api_key: None,
                },
            }),
            catalog: Arc::new(FakeCatalog),
            redactor: Arc::new(Redactor::new(16).expect("the test redactor builds")),
            redact_secrets: true,
            clock: fixed_time,
            next_id: test_id,
        })
    }
}

pub(crate) struct MockServer {
    pub(crate) base: String,
    tasks: JoinSet<Vec<String>>,
}

impl MockServer {
    pub(crate) async fn start(status: u16, body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the mock server binds");
        let address = listener
            .local_addr()
            .expect("the mock server has an address");
        let mut tasks = JoinSet::new();
        let answer = body.to_vec();
        tasks.spawn(serve(listener, status, answer));
        Self {
            base: format!("http://{address}"),
            tasks,
        }
    }

    pub(crate) async fn requests(mut self) -> Vec<String> {
        let joined = timeout(Duration::from_secs(10), self.tasks.join_next())
            .await
            .expect("the provider call arrives")
            .expect("the server task runs");
        joined.expect("the server task does not panic")
    }
}

async fn serve(listener: TcpListener, status: u16, answer: Vec<u8>) -> Vec<String> {
    let (mut socket, _) = listener.accept().await.expect("the mock server accepts");
    let request = read_request(&mut socket).await;
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        answer.len()
    );
    socket
        .write_all(head.as_bytes())
        .await
        .expect("the mock server writes");
    socket
        .write_all(&answer)
        .await
        .expect("the mock server writes");
    socket.shutdown().await.expect("the mock server closes");
    vec![request]
}

async fn read_request(socket: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    while !raw.ends_with(b"\r\n\r\n") {
        raw.push(socket.read_u8().await.expect("the mock server reads"));
    }
    let head = String::from_utf8(raw).expect("the request head is UTF-8");
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("a numeric length"))
        })
        .expect("the request declares its length");
    let mut bytes = vec![0_u8; length];
    socket
        .read_exact(&mut bytes)
        .await
        .expect("the mock server reads the body");
    let body = String::from_utf8(bytes).expect("the request body is UTF-8");
    format!("{head}\r\n{body}")
}

pub(crate) fn http_clients() -> HttpClients {
    let proxy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the test proxy policy resolves");
    let tls = TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test/0".to_owned(),
    };
    HttpClients::new(tls, Arc::new(proxy))
}

pub(super) fn ocr_client() -> OcrClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the test proxy policy resolves"),
    );
    let allowed = vec![String::from("127.0.0.1")];
    let validator = UrlValidator::new(SystemResolver, &allowed, &Verification::Enabled);
    OcrClient {
        provider_http: ProxiedClient::outbound(&policy).expect("the test client builds"),
        polling_http: ProxiedClient::outbound(&policy).expect("the test client builds"),
        document_http: ProxiedClient::outbound(&policy).expect("the test client builds"),
        document_fetcher: UserUrlFetcher {
            validator,
            validation: true,
        },
        settings: OcrSettings::default(),
        secrets: Arc::new(NoSecrets),
        azure: Arc::new(NoAzure),
        aws: Arc::new(NoAws),
        vertex: Arc::new(NoVertex),
    }
}

struct NoSecrets;

impl SecretSource for NoSecrets {
    fn secret(&self, _name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(None)
    }
}

struct NoAzure;

#[async_trait]
impl AzureTokenSource for NoAzure {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Ok(None)
    }
}

struct NoAws;

#[async_trait]
impl AwsRequestSigner for NoAws {
    async fn sign(
        &self,
        _request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        Ok(Vec::new())
    }
}

struct NoVertex;

#[async_trait]
impl VertexTokenSource for NoVertex {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        Err(OcrError::Unsupported("vertex access tokens"))
    }
}

pub(super) fn probe_logging() -> ProbeLogging {
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let init = LoggingInit {
        model: String::new(),
        call_type: String::from("acompletion"),
        start_time: fixed_time(),
        litellm_call_id: test_id(),
        function_id: test_id(),
        messages: Some(Messages::Values(Vec::new())),
        stream: Some(false),
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags {
            log_raw_request_response: true,
            raw_request_only: false,
        },
        callback_policy: CallbackPolicy::default(),
    };
    let redactor = Redactor::new(16).expect("the test redactor builds");
    let state = LoggingState::new(init, setup, &redactor, test_id, || Duration::ZERO)
        .expect("the test logging state builds");
    ProbeLogging::from(state)
}

pub(super) fn chat_body(headers: &[(&str, &str)]) -> Json {
    let entries: IndexMap<String, Json> = headers
        .iter()
        .map(|(name, value)| (String::from(*name), Json::Text(String::from(*value))))
        .collect();
    let mut hidden = IndexMap::new();
    hidden.insert(String::from("headers"), Json::Object(entries));
    let mut body = IndexMap::new();
    body.insert(String::from("_hidden_params"), Json::Object(hidden));
    Json::Object(body)
}

pub(super) fn headers_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

pub(super) fn mode_parameters(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), value.clone()))
        .collect()
}

pub(super) fn probe_call(parameters: IndexMap<String, Json>) -> ProbeCall {
    ProbeCall {
        parameters,
        api_key: Some(SecretString::from("sk-test")),
        logging: None,
    }
}

pub(crate) fn params(entries: &[(&str, Json)]) -> HealthProbeParams {
    let map = entries
        .iter()
        .map(|(key, value)| {
            (
                String::from(*key),
                serde_json::to_value(value).expect("the test parameter encodes"),
            )
        })
        .collect();
    HealthProbeParams::from_map(map).expect("the test parameters parse")
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(String::from(value))
}

pub(crate) fn json(value: &str) -> Json {
    serde_json::from_str(value).expect("the test JSON parses")
}

pub(super) fn document(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: String::from(source),
        extra: BTreeMap::new(),
    }
}

pub(crate) fn fixed_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(NOW).expect("the fixed time parses")
}

pub(super) fn test_id() -> String {
    Uuid::new_v4().to_string()
}

fn price(value: &str) -> Scalar {
    Scalar::Number(value.parse().expect("a test price parses"))
}

fn owner_failure(message: String) -> HealthProbeError {
    let source = io::Error::other(message.clone());
    HealthProbeError::Backend {
        class: plain_error(&message),
        message,
        source: Box::new(source),
    }
}
