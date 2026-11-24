#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_error::error::ErrorClass;
use betterlite_http::error::HttpError;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::{FileContentStreamingResult, StreamExit};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use uuid::Uuid;

use crate::api::file_stream_logging::logging::{
    CallbackFailure, FailureCall, FileStreamLogger, SuccessCall,
};
use crate::api::file_stream_logging::source::SourceFailure;
use crate::api::fine_tuning_read::{ReadStructuredTimeout, ReadTimeout, ReadTimeoutValue};
use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

use super::error::FileContentError;
use super::target::{
    CallerClient, ContentLogger, ContentLogging, ContentMode, ContentOperation, ContentReply,
    ContentTarget, FileContentDispatch, GenericContent, NewLogger, StreamIdentity, SuppliedSdk,
    XaiBatchContent,
};
use super::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
    StreamedContent,
};

const STARTED: &str = "2026-01-01T00:00:00+00:00";
const GENERATED_ID: &str = "00000000-0000-0000-0000-000000000001";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FakeConfig(ProviderId);

#[derive(Debug, Clone, PartialEq)]
struct GenericSeen {
    config: FakeConfig,
    api_key: Option<String>,
    api_base: Option<String>,
    trusted: Option<Vec<String>>,
    headers: BTreeMap<String, String>,
    created: Option<NewLogger>,
    client: Option<ClientKind>,
    mode: ContentMode,
}

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Compatible {
        api_base: String,
        api_key: Option<String>,
        organization: Option<String>,
        max_retries: Option<i64>,
        mode: ContentMode,
        client: Option<SuppliedSdk>,
    },
    Azure {
        api_base: Option<String>,
        api_key: Option<String>,
        api_version: Option<String>,
        max_retries: Option<i64>,
        mode: ContentMode,
        client: Option<SuppliedSdk>,
    },
    Generic(Box<GenericSeen>),
    XaiBatch {
        batch_id: String,
        api_base: Option<String>,
        api_key: Option<String>,
        timeout: FineTuningTimeout,
    },
}

impl Seen {
    fn mode(&self) -> ContentMode {
        match self {
            Self::Compatible { mode, .. } | Self::Azure { mode, .. } => *mode,
            Self::Generic(generic) => generic.mode,
            Self::XaiBatch { timeout, .. } => ContentMode {
                is_async: false,
                timeout: *timeout,
            },
        }
    }

    fn generic(&self) -> GenericSeen {
        match self {
            Self::Generic(generic) => (**generic).clone(),
            Self::Compatible { .. } | Self::Azure { .. } | Self::XaiBatch { .. } => {
                panic!("not a generic content call: {self:?}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Sent {
    target: Seen,
    request: FileContentRequest,
    chunk_size: Option<NonZeroUsize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FakeFailure(u16);

impl std::fmt::Display for FakeFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "provider status {}", self.0)
    }
}

impl std::error::Error for FakeFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedIdentity {
    model: String,
    custom_llm_provider: String,
    api_base: Option<String>,
}

// The logging object file_content_streaming annotates and the wrapper logs through
#[derive(Default)]
struct RecordingLogging {
    identities: Mutex<Vec<RecordedIdentity>>,
    successes: AtomicUsize,
    failures: AtomicUsize,
}

impl RecordingLogging {
    fn identities(&self) -> Vec<RecordedIdentity> {
        self.identities.lock().unwrap().clone()
    }

    fn successes(&self) -> usize {
        self.successes.load(Ordering::SeqCst)
    }

    fn failures(&self) -> usize {
        self.failures.load(Ordering::SeqCst)
    }
}

impl ContentLogging for RecordingLogging {
    fn record_identity(&self, identity: &StreamIdentity<'_>) {
        self.identities.lock().unwrap().push(RecordedIdentity {
            model: identity.model.to_owned(),
            custom_llm_provider: identity.custom_llm_provider.to_owned(),
            api_base: identity.api_base.map(str::to_owned),
        });
    }

    fn stream_logger(self: Arc<Self>) -> Arc<dyn FileStreamLogger> {
        self
    }
}

#[async_trait]
impl FileStreamLogger for RecordingLogging {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        None
    }

    fn details_api_base(&self) -> Option<String> {
        self.identities()
            .last()
            .and_then(|last| last.api_base.clone())
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        None
    }

    fn success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        self.successes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn async_success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        self.successes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn success_sync_callbacks(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        self.failures.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn async_failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }
}

struct FakeDispatch {
    configured: Vec<ProviderId>,
    sent: Mutex<Vec<Sent>>,
    created: Mutex<Vec<NewLogger>>,
    loggers: Mutex<Vec<Arc<RecordingLogging>>>,
    failure: Option<FakeFailure>,
    wrong_reply: bool,
}

impl FileContentDispatch for FakeDispatch {
    type Config = FakeConfig;
    type Client = ();
    type Logging = RecordingLogging;
    type Error = FakeFailure;

    fn files_config(&self, provider: ProviderId) -> Option<FakeConfig> {
        self.configured
            .contains(&provider)
            .then_some(FakeConfig(provider))
    }

    fn create_logging(&self, seed: &NewLogger) -> Arc<RecordingLogging> {
        let logger = Arc::new(RecordingLogging::default());
        self.created.lock().unwrap().push(seed.clone());
        self.loggers.lock().unwrap().push(Arc::clone(&logger));
        logger
    }

    fn content(
        &self,
        target: ContentTarget<'_, FakeConfig, ()>,
        operation: ContentOperation<'_>,
    ) -> impl Future<Output = Result<ContentReply, FakeFailure>> + Send {
        let (request, chunk_size, streamed) = match operation {
            ContentOperation::Buffered { request } => (request.clone(), None, false),
            ContentOperation::Streamed {
                request,
                chunk_size,
            } => (request.clone(), chunk_size, true),
        };
        self.sent.lock().unwrap().push(Sent {
            target: seen(target),
            request,
            chunk_size,
        });
        let reply = if streamed != self.wrong_reply {
            ContentReply::Streamed(Box::new(stream_result()))
        } else {
            ContentReply::Buffered(Box::new(binary()))
        };
        match self.failure {
            Some(failure) => ready(Err(failure)),
            None => ready(Ok(reply)),
        }
    }
}

fn seen(target: ContentTarget<'_, FakeConfig, ()>) -> Seen {
    match target {
        ContentTarget::Compatible(content) => Seen::Compatible {
            api_key: content
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            api_base: content.api_base,
            organization: content.organization,
            max_retries: content.max_retries,
            mode: content.mode,
            client: content.client,
        },
        ContentTarget::Azure(content) => Seen::Azure {
            api_key: content
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            api_base: content.api_base,
            api_version: content.api_version,
            max_retries: content.max_retries,
            mode: content.mode,
            client: content.client,
        },
        ContentTarget::Generic(boxed) => {
            let content: GenericContent<'_, FakeConfig, ()> = *boxed;
            Seen::Generic(Box::new(GenericSeen {
                config: content.config,
                api_key: content
                    .params
                    .api_key
                    .map(|secret| secret.expose_secret().to_owned()),
                api_base: content.params.api_base,
                trusted: content
                    .trusted
                    .map(|snapshot| snapshot.values.keys().cloned().collect()),
                headers: content.headers,
                created: match content.logger {
                    ContentLogger::Created(logger) => Some(*logger),
                    ContentLogger::Supplied(_) => None,
                },
                client: content.client.map(|supplied| supplied.kind),
                mode: content.mode,
            }))
        }
        ContentTarget::XaiBatch(content) => Seen::XaiBatch {
            batch_id: content.batch_id.to_owned(),
            api_base: content.api_base.map(str::to_owned),
            api_key: content
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            timeout: content.timeout,
        },
    }
}

fn binary() -> BinaryContent {
    BinaryContent {
        status: 200,
        headers: IndexMap::from([(
            String::from("content-type"),
            String::from("application/octet-stream"),
        )]),
        body: vec![0, 255],
    }
}

fn stream_result() -> FileContentStreamingResult {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-file"),
        HeaderValue::from_static("capture"),
    );
    FileContentStreamingResult::new(
        stream::iter(vec![Ok::<Vec<u8>, HttpError>(vec![0, 255])]).boxed(),
        &headers,
        None,
    )
}

// every model, catalog and environment value the resolver may read, held for the test's lifetime
struct Env {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }
}

impl Env {
    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

fn service<'deps>(
    resolution: &'deps ResolutionContext<'deps>,
    settings: Settings,
    configured: &[ProviderId],
) -> FileContentStream<'deps, FakeDispatch> {
    FileContentStream {
        dispatch: FakeDispatch {
            configured: configured.to_vec(),
            sent: Mutex::new(Vec::new()),
            created: Mutex::new(Vec::new()),
            loggers: Mutex::new(Vec::new()),
            failure: None,
            wrong_reply: false,
        },
        settings,
        resolution,
        clock: || DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap(),
        call_ids: || Uuid::from_u128(1),
    }
}

fn reader<'deps>(
    resolution: &'deps ResolutionContext<'deps>,
) -> FileContentStream<'deps, FakeDispatch> {
    service(
        resolution,
        Settings::default(),
        &[
            ProviderId::Anthropic,
            ProviderId::Bedrock,
            ProviderId::VertexAi,
        ],
    )
}

fn content_call<'call>(kwargs: &'call ContentKwargs) -> ContentCall<'call> {
    ContentCall {
        file_id: "file-1",
        model: None,
        custom_llm_provider: None,
        extra_headers: None,
        extra_body: None,
        chunk_size: 1024 * 1024,
        stream: false,
        kwargs,
    }
}

const fn bare() -> ContentContext<'static, (), RecordingLogging> {
    ContentContext {
        trusted: None,
        client: None,
        logger: None,
        logging: None,
    }
}

fn sent(read: &FileContentStream<'_, FakeDispatch>) -> Vec<Sent> {
    read.dispatch.sent.lock().unwrap().clone()
}

async fn run<'call>(
    read: &FileContentStream<'_, FakeDispatch>,
    entry: ContentEntry,
    call: &ContentCall<'call>,
    context: ContentContext<'call, (), RecordingLogging>,
) -> Result<ContentAnswer, FileContentError<FakeFailure>> {
    read.run(entry, call, context).await
}

fn answered_buffered(answer: ContentAnswer) -> BinaryContent {
    match answer {
        ContentAnswer::Buffered(content) => *content,
        ContentAnswer::Streamed(_) => panic!("not a buffered answer"),
    }
}

fn answered_streamed(answer: ContentAnswer) -> StreamedContent {
    match answer {
        ContentAnswer::Streamed(content) => *content,
        ContentAnswer::Buffered(_) => panic!("not a streamed answer"),
    }
}

async fn drained(content: &mut StreamedContent) -> Vec<Vec<u8>> {
    let mut iter = content.response.async_iter().unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = iter.next_chunk().await {
        chunks.push(chunk.unwrap());
    }
    chunks
}

fn text_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[tokio::test]
async fn file_content_defaults() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.file_id = "file-1";
    let answer = run(&read, ContentEntry::Async, &call, bare())
        .await
        .unwrap();
    let content = answered_buffered(answer);
    assert_eq!(content.body, vec![0, 255]);
    assert_eq!(content.status, 200);
    let expected_request = FileContentRequest {
        file_id: String::from("file-1"),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    assert_eq!(
        sent(&read),
        vec![Sent {
            target: Seen::Compatible {
                api_base: String::from("https://api.openai.com/v1"),
                api_key: None,
                organization: None,
                max_retries: None,
                mode: ContentMode {
                    is_async: true,
                    timeout: FineTuningTimeout::Seconds(600.0_f64),
                },
                client: None,
            },
            request: expected_request,
            chunk_size: None,
        }]
    );
}

#[tokio::test]
async fn file_content_defaults_sync_provider_is_none() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let failure = run(&read, ContentEntry::Sync, &content_call(&kwargs), bare())
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        FileContentError::UnknownProvider { provider } if provider == "None"
    ));
    assert_eq!(failure.to_string(), "None is not a valid LlmProviders");
    assert_eq!(failure.status(), None);
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn file_content_defaults_model_resolution_renames_the_provider() {
    let env = Env {
        names: BTreeSet::from([String::from("openai")]),
        ..Env::default()
    };
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = Some("openai/gpt-4o");
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert!(matches!(
        sent(&read).pop().unwrap().target,
        Seen::Compatible { .. }
    ));
}

#[tokio::test]
async fn file_content_defaults_resolution_errors_keep_the_declared_provider() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = Some("no-such-model");
    call.custom_llm_provider = Some(String::from("cohere"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        FileContentError::Unsupported { provider } if provider == "cohere"
    ));
    let mut unresolved = content_call(&kwargs);
    unresolved.model = Some("no-such-model");
    let swallowed = run(&read, ContentEntry::Sync, &unresolved, bare())
        .await
        .unwrap_err();
    assert!(matches!(
        &swallowed,
        FileContentError::UnknownProvider { provider } if provider == "None"
    ));
}

#[tokio::test]
async fn file_content_defaults_trusted_credentials_precede_key_and_base() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let trusted = TrustedCredentials {
        values: BTreeMap::from([(String::from("bucket"), SecretString::from("files"))]),
    };
    let mut kwargs = ContentKwargs::default();
    kwargs.api_key = Some(SecretString::from("sk-call"));
    kwargs.api_base = Some(String::from("https://call.example"));
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("anthropic"));
    let context = ContentContext {
        trusted: Some(&trusted),
        client: None,
        logger: None,
        logging: None,
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    let generic = sent(&read).pop().unwrap().target.generic();
    assert_eq!(generic.trusted, Some(vec![String::from("bucket")]));
    assert_eq!(generic.api_key.as_deref(), Some("sk-call"));
    assert_eq!(generic.api_base.as_deref(), Some("https://call.example"));

    let null_kwargs = ContentKwargs::default();
    let mut null_call = content_call(&null_kwargs);
    null_call.custom_llm_provider = Some(String::from("anthropic"));
    let null_context = ContentContext {
        trusted: Some(&trusted),
        client: None,
        logger: None,
        logging: None,
    };
    run(&read, ContentEntry::Sync, &null_call, null_context)
        .await
        .unwrap();
    let overwritten = sent(&read).pop().unwrap().target.generic();
    assert_eq!(overwritten.api_key, None);
    assert_eq!(overwritten.api_base, None);
    assert_eq!(overwritten.trusted, Some(vec![String::from("bucket")]));
}

fn number(value: u32) -> ReadTimeoutValue {
    ReadTimeoutValue::Number(Number::from(value))
}

fn text(value: &str) -> ReadTimeoutValue {
    ReadTimeoutValue::Text(value.to_owned())
}

fn phases(read: Option<u32>) -> ReadTimeout {
    ReadTimeout::Structured(ReadStructuredTimeout {
        connect: None,
        read: read.map(Number::from),
        write: None,
        pool: None,
    })
}

const fn kept(read: Option<f64>) -> FineTuningTimeout {
    FineTuningTimeout::Structured(StructuredTimeout {
        connect: None,
        read,
        write: None,
        pool: None,
    })
}

#[rstest]
#[case::timeout_over_request_timeout(
    Some(ReadTimeout::Plain(number(2))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(2.0_f64)
)]
#[case::request_timeout_alone(None, Some(number(3)), "openai", FineTuningTimeout::Seconds(3.0_f64))]
#[case::both_absent(None, None, "openai", FineTuningTimeout::Seconds(600.0_f64))]
#[case::zero_falls_through(
    Some(ReadTimeout::Plain(number(0))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(3.0_f64)
)]
#[case::empty_text_falls_through(
    Some(ReadTimeout::Plain(text(""))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(3.0_f64)
)]
#[case::numeric_text(
    Some(ReadTimeout::Plain(text("2.5"))),
    None,
    "openai",
    FineTuningTimeout::Seconds(2.5_f64)
)]
#[case::openai_keeps_phases(Some(phases(Some(5))), Some(number(3)), "openai", kept(Some(5.0_f64)))]
#[case::azure_keeps_phases(Some(phases(Some(5))), None, "azure", kept(Some(5.0_f64)))]
#[case::bedrock_keeps_phases(Some(phases(Some(5))), None, "bedrock", kept(Some(5.0_f64)))]
#[case::anthropic_reads_the_read_phase(
    Some(phases(Some(5))),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(5.0_f64)
)]
#[case::anthropic_without_read_phase(
    Some(phases(None)),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(600.0_f64)
)]
#[tokio::test]
async fn file_content_defaults_timeout_selection(
    #[case] timeout: Option<ReadTimeout>,
    #[case] request_timeout: Option<ReadTimeoutValue>,
    #[case] provider: &str,
    #[case] expected: FineTuningTimeout,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let mut kwargs = ContentKwargs::default();
    kwargs.timeout = timeout;
    kwargs.request_timeout = request_timeout;
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(sent(&read).pop().unwrap().target.mode().timeout, expected);
}

#[rstest]
#[case::unsupported_provider("cohere")]
#[case::unknown_provider("bogus")]
#[tokio::test]
async fn file_content_defaults_bad_text_fails_before_the_provider(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let mut kwargs = ContentKwargs::default();
    kwargs.timeout = Some(ReadTimeout::Plain(text("soon")));
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(matches!(&failure, FileContentError::Timeout(_)));
    assert_eq!(failure.status(), None);
    assert_eq!(sent(&read), Vec::new());
}

#[rstest]
#[case::openai("openai")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
#[tokio::test]
async fn buffered_content_dispatch_compatible(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let settings = Settings {
        openai_api_key: Some(SecretString::from("sk-settings")),
        openai_organization: Some(String::from("org-settings")),
        ..Settings::default()
    };
    let read = service(&resolution, settings, &[]);
    let mut kwargs = ContentKwargs::default();
    kwargs.max_retries = Some(4);
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    call.extra_headers = Some(text_map(&[("x-trace", "1")]));
    call.extra_body = Some(text_map(&[("dropped", "1")]));
    let answer = run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(answered_buffered(answer).body, vec![0, 255]);
    let record = sent(&read).pop().unwrap();
    assert_eq!(
        record.target,
        Seen::Compatible {
            api_base: String::from("https://api.openai.com/v1"),
            api_key: Some(String::from("sk-settings")),
            organization: Some(String::from("org-settings")),
            max_retries: Some(4),
            mode: ContentMode {
                is_async: false,
                timeout: FineTuningTimeout::Seconds(600.0_f64),
            },
            client: None,
        }
    );
    assert_eq!(
        record.request.extra_headers,
        IndexMap::from([(String::from("x-trace"), String::from("1"))])
    );
    assert_eq!(
        record.request.extra_body,
        IndexMap::from([(String::from("dropped"), String::from("1"))])
    );
    assert_eq!(record.request.timeout, None);
}

#[tokio::test]
async fn buffered_content_dispatch_azure() {
    let env = Env::default();
    let resolution = env.resolution();
    let settings = Settings {
        azure_api_key: Some(SecretString::from("sk-azure")),
        azure_api_version: Some(String::from("2024-06-01")),
        ..Settings::default()
    };
    let read = service(&resolution, settings, &[]);
    let mut kwargs = ContentKwargs::default();
    kwargs.api_base = Some(String::from("https://azure.example"));
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("azure"));
    let context = ContentContext {
        trusted: None,
        client: Some(CallerClient::Sdk(SuppliedSdk::Azure)),
        logger: None,
        logging: None,
    };
    run(&read, ContentEntry::Async, &call, context)
        .await
        .unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target,
        Seen::Azure {
            api_base: Some(String::from("https://azure.example")),
            api_key: Some(String::from("sk-azure")),
            api_version: Some(String::from("2024-06-01")),
            max_retries: None,
            mode: ContentMode {
                is_async: true,
                timeout: FineTuningTimeout::Seconds(600.0_f64),
            },
            client: Some(SuppliedSdk::Azure),
        }
    );
}

#[rstest]
#[case::anthropic(ProviderId::Anthropic)]
#[case::bedrock(ProviderId::Bedrock)]
#[case::vertex_ai(ProviderId::VertexAi)]
#[tokio::test]
async fn buffered_content_dispatch_generic_configs(#[case] provider: ProviderId) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let trusted = TrustedCredentials {
        values: BTreeMap::from([(String::from("bucket"), SecretString::from("files"))]),
    };
    let mut kwargs = ContentKwargs::default();
    kwargs.api_key = Some(SecretString::from("sk-call"));
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_string());
    let context = ContentContext {
        trusted: Some(&trusted),
        client: Some(CallerClient::Http(SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &(),
        })),
        logger: None,
        logging: None,
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    let generic = sent(&read).pop().unwrap().target.generic();
    assert_eq!(generic.config, FakeConfig(provider));
    assert_eq!(generic.client, Some(ClientKind::Asynchronous));
    assert_eq!(generic.trusted, Some(vec![String::from("bucket")]));
    assert_eq!(generic.api_key.as_deref(), Some("sk-call"));
    assert_eq!(
        generic.created,
        Some(NewLogger {
            call_type: "file_content",
            start_time: DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap(),
            litellm_call_id: String::from(GENERATED_ID),
            function_id: String::new(),
        })
    );
}

#[tokio::test]
async fn buffered_content_dispatch_generic_drops_sdk_clients() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("anthropic"));
    let context = ContentContext {
        trusted: None,
        client: Some(CallerClient::Sdk(SuppliedSdk::OpenAi)),
        logger: None,
        logging: None,
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(sent(&read).pop().unwrap().target.generic().client, None);
}

#[tokio::test]
async fn buffered_content_dispatch_compatible_drops_every_client() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    let context = ContentContext {
        trusted: None,
        client: Some(CallerClient::Sdk(SuppliedSdk::OpenAi)),
        logger: None,
        logging: None,
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target,
        Seen::Compatible {
            api_base: String::from("https://api.openai.com/v1"),
            api_key: None,
            organization: None,
            max_retries: None,
            mode: ContentMode {
                is_async: false,
                timeout: FineTuningTimeout::Seconds(600.0_f64),
            },
            client: None,
        }
    );
}

#[tokio::test]
async fn buffered_content_dispatch_xai_batch_precedes_config() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = service(
        &resolution,
        Settings::default(),
        &[ProviderId::Xai, ProviderId::Anthropic],
    );
    let mut kwargs = ContentKwargs::default();
    kwargs.api_key = Some(SecretString::from("sk-xai"));
    kwargs.api_base = Some(String::from("https://xai.example"));
    let mut call = content_call(&kwargs);
    call.file_id = "batch_7";
    call.custom_llm_provider = Some(String::from("xai"));
    run(&read, ContentEntry::Async, &call, bare())
        .await
        .unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target,
        Seen::XaiBatch {
            batch_id: String::from("batch_7"),
            api_base: Some(String::from("https://xai.example")),
            api_key: Some(String::from("sk-xai")),
            timeout: FineTuningTimeout::Seconds(600.0_f64),
        }
    );
}

#[tokio::test]
async fn buffered_content_dispatch_ordinary_xai_file_uses_its_config() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = service(&resolution, Settings::default(), &[ProviderId::Xai]);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.file_id = "file-7";
    call.custom_llm_provider = Some(String::from("xai"));
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target.generic().config,
        FakeConfig(ProviderId::Xai)
    );
}

#[rstest]
#[case::not_found(404)]
#[case::rate_limited(429)]
#[tokio::test]
async fn buffered_content_dispatch_provider_errors_survive(#[case] status: u16) {
    let env = Env::default();
    let resolution = env.resolution();
    let mut read = reader(&resolution);
    read.dispatch.failure = Some(FakeFailure(status));
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(
        matches!(&failure, FileContentError::Dispatch(FakeFailure(answered)) if *answered == status)
    );
    assert_eq!(failure.status(), None);
    assert_eq!(sent(&read).len(), 1);
}

fn written_identity(model: &str, provider: &str, api_base: Option<&str>) -> RecordedIdentity {
    RecordedIdentity {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        api_base: api_base.map(str::to_owned),
    }
}

#[tokio::test]
async fn streaming_logging_identity_keeps_a_supplied_object() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let mut kwargs = ContentKwargs::default();
    kwargs.api_base = Some(String::from("https://call.example"));
    let mut call = content_call(&kwargs);
    call.model = Some("gpt-4o");
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let context = ContentContext {
        trusted: None,
        client: None,
        logger: None,
        logging: Some(Arc::clone(&logger)),
    };
    let mut content = answered_streamed(
        run(&read, ContentEntry::Async, &call, context)
            .await
            .unwrap(),
    );
    assert!(read.dispatch.created.lock().unwrap().is_empty());
    assert_eq!(
        logger.identities(),
        vec![written_identity(
            "gpt-4o",
            "openai",
            Some("https://call.example")
        )]
    );
    assert_eq!(
        content.headers.get("x-file").map(String::as_str),
        Some("capture")
    );
    assert_eq!(drained(&mut content).await, vec![vec![0, 255]]);
    assert_eq!(logger.successes(), 1);
    assert_eq!(logger.failures(), 0);
    assert_eq!(content.exit.get(), Some(&StreamExit::Completed));
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), Some(""))]
#[case::model(Some("gpt-4o"), Some("gpt-4o"))]
#[tokio::test]
async fn streaming_logging_identity_normalizes_the_model(
    #[case] model: Option<&str>,
    #[case] written: Option<&str>,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = model;
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let context = ContentContext {
        trusted: None,
        client: None,
        logger: None,
        logging: Some(Arc::clone(&logger)),
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(
        logger.identities(),
        vec![written_identity(
            written.unwrap_or_default(),
            "openai",
            None
        )]
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::value(Some("https://call.example"))]
#[tokio::test]
async fn streaming_logging_identity_writes_only_a_non_null_api_base(
    #[case] api_base: Option<&str>,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let mut kwargs = ContentKwargs::default();
    kwargs.api_base = api_base.map(str::to_owned);
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("hosted_vllm"));
    call.stream = true;
    let context = ContentContext {
        trusted: None,
        client: None,
        logger: None,
        logging: Some(Arc::clone(&logger)),
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(
        logger.identities(),
        vec![written_identity("", "hosted_vllm", api_base)]
    );
}

#[rstest]
#[case::generated(None, None, "afile_content", GENERATED_ID, "")]
#[case::given_call_id(Some("call-7"), None, "afile_content", "call-7", "")]
#[case::empty_call_id(Some(""), None, "afile_content", GENERATED_ID, "")]
#[case::given_function_id(None, Some("fn-1"), "afile_content", GENERATED_ID, "fn-1")]
#[case::sync_call_type(None, None, "file_content", GENERATED_ID, "")]
#[tokio::test]
async fn streaming_logging_identity_creates_the_missing_object(
    #[case] litellm_call_id: Option<&str>,
    #[case] id: Option<&str>,
    #[case] call_type: &'static str,
    #[case] call_id: &str,
    #[case] function_id: &str,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let mut kwargs = ContentKwargs::default();
    kwargs.litellm_call_id = litellm_call_id.map(str::to_owned);
    kwargs.id = id.map(str::to_owned);
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let entry = if call_type == "afile_content" {
        ContentEntry::Async
    } else {
        ContentEntry::Sync
    };
    run(&read, entry, &call, bare()).await.unwrap();
    assert_eq!(
        *read.dispatch.created.lock().unwrap(),
        vec![NewLogger {
            call_type,
            start_time: DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap(),
            litellm_call_id: call_id.to_owned(),
            function_id: function_id.to_owned(),
        }]
    );
    let logger = read.dispatch.loggers.lock().unwrap()[0].clone();
    assert_eq!(
        logger.identities(),
        vec![written_identity("", "openai", None)]
    );
}

#[tokio::test]
async fn streaming_logging_identity_precedes_the_vertex_error() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = Some("gemini-2.0-flash");
    call.custom_llm_provider = Some(String::from("vertex_ai"));
    call.stream = true;
    let context = ContentContext {
        trusted: None,
        client: None,
        logger: None,
        logging: Some(Arc::clone(&logger)),
    };
    let failure = run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap_err();
    assert!(matches!(failure, FileContentError::SyncVertexStreaming));
    assert_eq!(
        logger.identities(),
        vec![written_identity("gemini-2.0-flash", "vertex_ai", None)]
    );
    assert_eq!(sent(&read), Vec::new());
}

#[rstest]
#[case::openai("openai")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
#[tokio::test]
async fn sdk_stream_selection_compatible(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    call.stream = true;
    call.chunk_size = 3;
    let context = ContentContext {
        trusted: None,
        client: Some(CallerClient::Sdk(SuppliedSdk::OpenAi)),
        logger: None,
        logging: None,
    };
    let mut content = answered_streamed(
        run(&read, ContentEntry::Async, &call, context)
            .await
            .unwrap(),
    );
    assert_eq!(drained(&mut content).await, vec![vec![0, 255]]);
    let record = sent(&read).pop().unwrap();
    assert_eq!(record.chunk_size, NonZeroUsize::new(3));
    assert!(matches!(
        record.target,
        Seen::Compatible {
            client: Some(SuppliedSdk::OpenAi),
            ..
        }
    ));
}

#[tokio::test]
async fn sdk_stream_selection_default_chunk_size_and_no_sdk_azure_client() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let context = ContentContext {
        trusted: None,
        client: Some(CallerClient::Sdk(SuppliedSdk::Azure)),
        logger: None,
        logging: None,
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    let record = sent(&read).pop().unwrap();
    assert_eq!(record.chunk_size, NonZeroUsize::new(1024 * 1024));
    assert!(matches!(
        record.target,
        Seen::Compatible { client: None, .. }
    ));
}

#[tokio::test]
async fn sdk_stream_selection_zero_chunk_size_takes_the_default() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    call.chunk_size = 0;
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(sent(&read).pop().unwrap().chunk_size, None);
}

#[tokio::test]
async fn sdk_stream_selection_vertex_needs_an_async_http_handler() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("vertex_ai"));
    call.stream = true;
    let context = ContentContext {
        trusted: None,
        client: Some(CallerClient::Http(SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &(),
        })),
        logger: None,
        logging: None,
    };
    answered_streamed(
        run(&read, ContentEntry::Async, &call, context)
            .await
            .unwrap(),
    );
    let record = sent(&read).pop().unwrap();
    assert_eq!(
        record.target.generic().client,
        Some(ClientKind::Asynchronous)
    );
    assert_eq!(
        record.target.generic().config,
        FakeConfig(ProviderId::VertexAi)
    );

    let sync_client = ContentContext {
        trusted: None,
        client: Some(CallerClient::Http(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &(),
        })),
        logger: None,
        logging: None,
    };
    answered_streamed(
        run(&read, ContentEntry::Async, &call, sync_client)
            .await
            .unwrap(),
    );
    assert_eq!(sent(&read).pop().unwrap().target.generic().client, None);
}

#[rstest]
#[case::azure("azure")]
#[case::bedrock("bedrock")]
#[case::anthropic("anthropic")]
#[tokio::test]
async fn sdk_stream_selection_unsupported_names_stay_buffered(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    call.stream = true;
    call.chunk_size = 3;
    let answer = run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    answered_buffered(answer);
    let record = sent(&read).pop().unwrap();
    assert_eq!(record.chunk_size, None);
}

#[tokio::test]
async fn sdk_stream_selection_sync_vertex_rejects_the_placeholder() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("vertex_ai"));
    call.stream = true;
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Streaming 'file_content' for vertex_ai is only supported through 'afile_content'."
    );
    let placeholder = failure.bad_request().unwrap();
    assert_eq!(placeholder.model, "n/a");
    assert_eq!(placeholder.body, "Unsupported provider");
    assert_eq!(placeholder.method, "FILE_CONTENT");
    assert_eq!(placeholder.llm_provider, "vertex_ai");
    assert_eq!(placeholder.url, "https://github.com/BerriAI/litellm");
    assert_eq!(failure.status(), Some(400));
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn sdk_stream_selection_unreachable_helper_branch_is_still_built() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let call = content_call(&kwargs);
    let request = FileContentRequest {
        file_id: String::from("file-1"),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let mode = ContentMode {
        is_async: true,
        timeout: FineTuningTimeout::Seconds(600.0_f64),
    };
    let failure = read
        .streamed(&call, Some(String::from("cohere")), mode, request, bare())
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        FileContentError::UnsupportedStreaming { provider } if provider == "cohere"
    ));
    assert_eq!(
        failure.to_string(),
        "LiteLLM doesn't support cohere for streaming 'file_content'. Supported providers are ['hosted_vllm', 'litellm_proxy', 'openai', 'vertex_ai']."
    );
    assert_eq!(failure.status(), Some(400));
    assert_eq!(sent(&read), Vec::new());

    let unconfigured = service(&resolution, Settings::default(), &[]);
    let another = FileContentRequest {
        file_id: String::from("file-1"),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let vertex = unconfigured
        .streamed(
            &call,
            Some(String::from("vertex_ai")),
            mode,
            another,
            bare(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        &vertex,
        FileContentError::UnsupportedStreaming { provider } if provider == "vertex_ai"
    ));
    assert_eq!(sent(&unconfigured), Vec::new());
}

#[tokio::test]
async fn content_reply_kinds_are_checked() {
    let env = Env::default();
    let resolution = env.resolution();
    let mut read = reader(&resolution);
    read.dispatch.wrong_reply = true;
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    let buffered = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(matches!(buffered, FileContentError::Reply));
    call.stream = true;
    let streamed = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(matches!(streamed, FileContentError::Reply));
}

#[tokio::test]
async fn unsupported_file_content() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("cohere"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        FileContentError::Unsupported { provider } if provider == "cohere"
    ));
    assert_eq!(
        failure.to_string(),
        "LiteLLM doesn't support cohere for 'file_content'. Supported providers are 'openai', 'azure', 'vertex_ai', 'bedrock', 'manus', 'anthropic'."
    );
    let placeholder = failure.bad_request().unwrap();
    assert_eq!(placeholder.model, "n/a");
    assert_eq!(placeholder.llm_provider, "cohere");
    assert_eq!(placeholder.body, "Unsupported provider");
    assert_eq!(placeholder.method, "CREATE_THREAD");
    assert_eq!(placeholder.url, "https://github.com/BerriAI/litellm");
    assert_eq!(failure.status(), Some(400));
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn unsupported_file_content_unknown_provider_text() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("bogus"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        FileContentError::UnknownProvider { provider } if provider == "'bogus'"
    ));
    assert_eq!(failure.to_string(), "'bogus' is not a valid LlmProviders");
    assert_eq!(failure.status(), None);
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn unsupported_file_content_stream_flag_does_not_change_the_branch() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("cohere"));
    call.stream = true;
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .unwrap_err();
    assert!(matches!(failure, FileContentError::Unsupported { .. }));
}

#[tokio::test]
async fn buffered_content_dispatch_keeps_a_supplied_logging_state() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"files","call_type":"file_content","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    )
    .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap();
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("anthropic"));
    let context = ContentContext {
        trusted: None,
        client: None,
        logger: Some(&mut state),
        logging: None,
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(sent(&read).pop().unwrap().target.generic().created, None);
}

// The error class the wrapper logs, which LiteLLM's _stream raises unchanged
#[test]
fn streamed_source_failures_are_unclassified_exceptions() {
    let failure = super::service::source_failure(FilesError::NotAsync);
    let SourceFailure::Exception(ErrorClass::Unclassified(context)) = &failure else {
        panic!("not an unclassified exception: {failure:?}");
    };
    assert!(context.message.contains("AsyncOpenAI"));
}
