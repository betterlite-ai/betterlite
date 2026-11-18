use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_embedding_io::credentials::ManagedRegionWorker;
use betterlite_bedrock::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, Timeout, TransferError,
};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::embedding_jobs::{
    AsyncInvokeStatusCall, EmbeddingJobsError, StatusCredentials, StatusReply, StatusServices,
    StatusTransport, StatusWireRequest, get_async_invoke_status,
};
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::Failure;
use super::input::Call;

const CLOCK_SECONDS: u64 = 1_700_000_000;

pub(crate) struct Execution {
    pub(crate) result: Result<Json, EmbeddingJobsError>,
    pub(crate) requests: Vec<StatusWireRequest>,
    pub(crate) factory_calls: Vec<(String, Option<Timeout>)>,
    pub(crate) resolved_regions: Vec<String>,
    pub(crate) snapshots: Vec<Json>,
}

struct Source {
    credentials: Credentials,
    regions: Mutex<Vec<String>>,
}

impl StatusCredentials for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.regions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(region);
        ready(Ok(self.credentials.clone()))
    }
}

struct FixtureClient {
    status: u16,
    body: String,
    seen: Arc<Mutex<Vec<StatusWireRequest>>>,
}

impl StatusTransport for FixtureClient {
    fn get(
        &self,
        request: StatusWireRequest,
    ) -> impl Future<Output = Result<StatusReply, TransferError>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        ready(Ok(StatusReply {
            status: self.status,
            body: self.body.as_bytes().to_vec(),
        }))
    }
}

// B173's ClientFactory needs a post for the embedding calls this task does not make
impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        _request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        ready(Err(TransferError::Failed(TransportError::Closed)))
    }
}

struct Factory {
    status: u16,
    body: String,
    seen: Arc<Mutex<Vec<StatusWireRequest>>>,
    calls: Mutex<Vec<(String, Option<Timeout>)>>,
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, _timeout: Option<Timeout>) -> Self::Client {
        FixtureClient {
            status: self.status,
            body: self.body.clone(),
            seen: Arc::clone(&self.seen),
        }
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((provider.to_owned(), timeout));
        FixtureClient {
            status: self.status,
            body: self.body.clone(),
            seen: Arc::clone(&self.seen),
        }
    }
}

struct NoPorts;

impl OidcHttp for NoPorts {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoPorts {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoPorts {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}

#[derive(Default)]
struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

fn redactor() -> Result<Redactor, Failure> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|_| Failure::Fixture)
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, Failure> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"embedding-model","call_type":"embeddings","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .map_err(|_| Failure::Fixture)
}

fn parameters(call: &Call) -> Result<BTreeMap<String, Box<RawValue>>, Failure> {
    let mut parameters = BTreeMap::new();
    parameters.insert(
        "aws_access_key_id".to_owned(),
        RawValue::from_string(serde_json::to_string(&call.credentials.access_key_id)?)?,
    );
    parameters.insert(
        "aws_secret_access_key".to_owned(),
        RawValue::from_string(serde_json::to_string(&call.credentials.secret_access_key)?)?,
    );
    if let Some(token) = &call.credentials.session_token {
        parameters.insert(
            "aws_session_token".to_owned(),
            RawValue::from_string(serde_json::to_string(token)?)?,
        );
    }
    if let Some(endpoint) = &call.endpoint {
        parameters.insert(
            "aws_bedrock_runtime_endpoint".to_owned(),
            RawValue::from_string(serde_json::to_string(endpoint)?)?,
        );
    }
    Ok(parameters)
}

pub(crate) fn execute(call: &Call) -> Result<Execution, Failure> {
    let source = Source {
        credentials: Credentials {
            access_key_id: SecretString::from(call.credentials.access_key_id.clone()),
            secret_access_key: SecretString::from(call.credentials.secret_access_key.clone()),
            session_token: call
                .credentials
                .session_token
                .clone()
                .map(SecretString::from),
        },
        regions: Mutex::new(Vec::new()),
    };
    let secrets = secrets();
    let region_worker = ManagedRegionWorker::new().map_err(|_| Failure::Fixture)?;
    let seen: Arc<Mutex<Vec<StatusWireRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let factory = Factory {
        status: call.status,
        body: call.body.clone(),
        seen: Arc::clone(&seen),
        calls: Mutex::new(Vec::new()),
    };
    let mut parameters = parameters(call)?;
    let redactor = redactor()?;
    let mut state = logging_state(&redactor)?;
    let snapshots: Arc<Mutex<Vec<Json>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&snapshots);
    let logger = move |snapshot: &Json| -> Result<(), PrePostError> {
        captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(snapshot.clone());
        Ok(())
    };
    let sink = DiscardingSink;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(&logger),
        now: state.start_time,
    };
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let now = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(CLOCK_SECONDS))
        .ok_or(Failure::Fixture)?;
    let request = AsyncInvokeStatusCall {
        invocation_arn: &call.invocation_arn,
        region: &call.region,
        optional_params: &mut parameters,
        environment: &call.environment,
        now,
    };
    let services = StatusServices {
        source: &source,
        secrets: &secrets,
        region_worker: &region_worker,
        factory: &factory,
    };
    let logging = call.logging.then_some(&mut log);
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Failure::Fixture)?
        .block_on(get_async_invoke_status(request, &services, logging));
    Ok(Execution {
        result,
        requests: seen.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        factory_calls: factory
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        resolved_regions: source
            .regions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        snapshots: snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    })
}
