use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::image_call::plan::CredentialSource;
use betterlite_bedrock::image_call::transport::{
    ClientFactory, ImageReply, ImageRequest, ImageTransport, TransferError,
};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

// 1_700_000_000 seconds is 2023-11-14T22:13:20Z, the instant the composition fixtures sign at
pub const NOW_SECONDS: i64 = 1_700_000_000;

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("the fixture instant is outside the clock")]
    Instant,
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Redact(#[from] RedactionError),
}

pub fn instant(seconds: i64) -> Result<SystemTime, FixtureError> {
    let whole = u64::try_from(seconds).map_err(|_error| FixtureError::Instant)?;
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(whole))
        .ok_or(FixtureError::Instant)
}

pub type Seen = Arc<Mutex<Vec<ImageRequest>>>;

// What the fake upstream answers: an ImageReply, a timeout, or a closed transport
#[derive(Clone, Debug)]
pub enum Reply {
    Answer(ImageReply),
    Timeout,
    Failed,
}

// Stands in for both an HTTPHandler and an AsyncHTTPHandler and records every request it was handed
pub struct FixtureClient {
    pub reply: Reply,
    pub seen: Seen,
}

impl ImageTransport for FixtureClient {
    fn post(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransferError>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let reply = match &self.reply {
            Reply::Answer(reply) => Ok(reply.clone()),
            Reply::Timeout => Err(TransferError::Timeout),
            Reply::Failed => Err(TransferError::Failed(TransportError::Closed)),
        };
        ready(reply)
    }
}

// B173 builds the client a leaf asks for; the factory records which kind and timeout it was asked for
pub struct Factory {
    pub reply: Reply,
    pub calls: Mutex<Vec<(ClientKind, Option<Duration>)>>,
    pub seen: Seen,
}

impl Factory {
    fn client(&self, kind: ClientKind, timeout: Option<Duration>) -> FixtureClient {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((kind, timeout));
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self) -> Self::Client {
        self.client(ClientKind::Sync, None)
    }

    fn async_client(&self, timeout: Option<Duration>) -> Self::Client {
        self.client(ClientKind::Asynchronous, timeout)
    }
}

// The credential lookup B595 would run; the fixture answers with the one identity it was built with
pub struct Source {
    pub credentials: Credentials,
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(self.credentials.clone()))
    }
}

#[derive(Default)]
pub struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub fn logging_state(redactor: &Redactor) -> Result<LoggingState, FixtureError> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"amazon.titan-image-generator-v2:0","call_type":"image_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    Ok(LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )?)
}

#[must_use]
pub fn prepost_env<'env>(
    state: &LoggingState,
    redactor: &'env Redactor,
    sink: &'env DiscardingSink,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: state.start_time,
    }
}

// The typed catalog stays empty: the edit selector reads the raw model-cost map the fixture supplies
pub struct Catalog {
    typed: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Catalog {
    pub fn empty() -> Result<Self, FixtureError> {
        Ok(Self {
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}
