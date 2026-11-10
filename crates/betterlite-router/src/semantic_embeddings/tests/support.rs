use std::collections::BTreeMap;
use std::future::{Future, pending};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use tokio::time::Instant;

use crate::semantic_embedding_inputs::ConfiguredTokenLimits;
use crate::semantic_embeddings::error::SemanticEmbeddingError;
use crate::semantic_embeddings::{
    EmbeddedReply, EmbeddingCalls, EmbeddingFailure, EmbeddingMode, EmbeddingRequest, MetadataMap,
    ModelListEntry, SemanticEmbeddingService, SemanticEmbeddingSettings,
};

pub(crate) const MODEL: &str = "text-embedding-3-small";
pub(crate) use betterlite_config::sections::semantic_embedding_inputs::DEFAULT_TIMEOUT_SECONDS as DEFAULT_TIMEOUT;

pub(crate) async fn qdrant_embedding(
    service: &Service,
    prompt: &str,
    metadata: Option<&MetadataMap>,
    mode: EmbeddingMode,
) -> Result<TestResponse, SemanticEmbeddingError<TestCallError>> {
    let request = service.prepare(prompt, metadata).await?;
    service.call(&request, mode).await
}

pub(crate) type Service = SemanticEmbeddingService<TestRouter, RecordingCalls>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TestResponse {
    pub(crate) vectors: Vec<Vec<f64>>,
}

impl EmbeddedReply for TestResponse {
    fn first_embedding(&self) -> Option<Vec<f64>> {
        self.vectors.first().cloned()
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("the scripted embedding call failed")]
pub(crate) struct TestCallError {
    status: Option<u16>,
}

impl EmbeddingFailure for TestCallError {
    fn status_code(&self) -> Option<u16> {
        self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallMethod {
    Sync,
    Async,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallOutcome {
    Succeed,
    Fail,
    Hang,
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedCall {
    pub(crate) method: CallMethod,
    pub(crate) request: EmbeddingRequest,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CallLog {
    entries: Arc<Mutex<Vec<RecordedCall>>>,
}

impl CallLog {
    pub(crate) fn recorded(&self) -> Vec<RecordedCall> {
        self.entries
            .lock()
            .expect("the call log is not poisoned")
            .clone()
    }
}

#[derive(Clone)]
pub(crate) struct RecordingCalls {
    pub(crate) log: CallLog,
    response: TestResponse,
    outcome: CallOutcome,
    status: Option<u16>,
}

impl RecordingCalls {
    pub(crate) fn succeeding(vectors: Vec<Vec<f64>>) -> Self {
        Self::scripted(CallOutcome::Succeed, vectors, None)
    }

    pub(crate) fn failing(status: u16) -> Self {
        Self::scripted(CallOutcome::Fail, Vec::new(), Some(status))
    }

    pub(crate) fn hanging() -> Self {
        Self::scripted(CallOutcome::Hang, Vec::new(), None)
    }

    fn scripted(outcome: CallOutcome, vectors: Vec<Vec<f64>>, status: Option<u16>) -> Self {
        Self {
            log: CallLog::default(),
            response: TestResponse { vectors },
            outcome,
            status,
        }
    }

    pub(crate) fn recorded(&self) -> Vec<RecordedCall> {
        self.log.recorded()
    }

    fn record(&self, method: CallMethod, request: &EmbeddingRequest) {
        self.log
            .entries
            .lock()
            .expect("the call log is not poisoned")
            .push(RecordedCall {
                method,
                request: request.clone(),
            });
    }

    fn respond(
        &self,
        method: CallMethod,
        request: &EmbeddingRequest,
    ) -> impl Future<Output = Result<TestResponse, TestCallError>> + Send {
        self.record(method, request);
        let outcome = self.outcome;
        let response = self.response.clone();
        let status = self.status;
        async move {
            match outcome {
                CallOutcome::Succeed => Ok(response),
                CallOutcome::Fail => Err(TestCallError { status }),
                CallOutcome::Hang => pending().await,
            }
        }
    }
}

impl EmbeddingCalls for RecordingCalls {
    type Response = TestResponse;
    type Error = TestCallError;

    fn embedding(
        &self,
        request: &EmbeddingRequest,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send {
        self.respond(CallMethod::Sync, request)
    }

    fn aembedding(
        &self,
        request: &EmbeddingRequest,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send {
        self.respond(CallMethod::Async, request)
    }
}

pub(crate) struct TestRouter {
    max_input_tokens: Option<i64>,
}

impl ConfiguredTokenLimits for TestRouter {
    fn get_configured_token_limits(&self, _model_name: &str) -> (Option<i64>, Option<i64>) {
        (self.max_input_tokens, None)
    }
}

pub(crate) fn router(max_input_tokens: Option<i64>) -> TestRouter {
    TestRouter { max_input_tokens }
}

pub(crate) fn model_list(model_name: &str) -> Vec<ModelListEntry> {
    vec![ModelListEntry {
        model_name: Some(model_name.to_owned()),
    }]
}

pub(crate) fn settings(
    embedding_max_input_tokens: Option<i64>,
    embedding_timeout: Option<f64>,
) -> SemanticEmbeddingSettings {
    SemanticEmbeddingSettings {
        embedding_model: MODEL.to_owned(),
        embedding_max_input_tokens,
        embedding_timeout,
        default_timeout_seconds: DEFAULT_TIMEOUT,
    }
}

pub(crate) fn service(
    settings: SemanticEmbeddingSettings,
    router: Option<TestRouter>,
    model_list: Option<Vec<ModelListEntry>>,
    calls: &RecordingCalls,
    selector: Arc<Selector>,
) -> Service {
    SemanticEmbeddingService::new(settings, router, model_list, calls.clone(), selector)
}

pub(crate) fn selector() -> Selector {
    selector_with(Arc::new(InlineCounter))
}

pub(crate) fn failing_selector() -> Selector {
    selector_with(Arc::new(FailingCounter))
}

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

struct FailingCounter;

#[async_trait]
impl BlockingCounter for FailingCounter {
    async fn count(&self, _job: CountJob) -> Result<usize, EngineError> {
        Err(EngineError::PoolClosed)
    }

    async fn build(&self, _job: BuildJob) -> Result<Engine, EngineError> {
        Err(EngineError::PoolClosed)
    }

    async fn convert(&self, _job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        Err(EngineError::PoolClosed)
    }
}

fn selector_with(facility: Arc<dyn BlockingCounter>) -> Selector {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the default proxy policy resolves");
    let client =
        Arc::new(ProxiedClient::outbound(&Arc::new(policy)).expect("the test client builds"));
    let settings = HubSettings {
        endpoint: "https://huggingface.co".to_owned(),
        offline: OfflineSetting::Offline,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: PathBuf::new(),
        hub_cache: PathBuf::new(),
        token_path: PathBuf::new(),
        stored_tokens_path: PathBuf::new(),
        transfer: TransferSetting::Standard,
    };
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let tokens = Arc::new(HubTokenResolver::new(
        settings.clone(),
        OidcSettings::default(),
        Arc::clone(&client),
        reader,
        clock,
    ));
    let hub = Arc::new(HubDownloader::new(
        Arc::new(settings),
        tokens,
        Arc::new(Instant::now),
    ));
    let counter = Arc::new(CountingService::new(4, facility, EncodeLimits::default()));
    let fetcher = Arc::new(TiktokenFetcher::new(client, None, Arc::clone(&counter)));
    let catalog = BundledCatalog::default().get();
    Selector::new(fetcher, hub, catalog, counter, false)
}
