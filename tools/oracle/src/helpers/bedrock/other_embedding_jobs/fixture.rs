use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_embedding_io::credentials::ManagedRegionWorker;
use betterlite_bedrock::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, Timeout, TransferError,
};
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::embedding_jobs::error::EmbeddingJobsError;
use betterlite_bedrock::embedding_jobs::logging::StatusLogger;
use betterlite_bedrock::embedding_jobs::status::{
    AsyncInvokeStatusCall, StatusServices, get_async_invoke_status,
};
use betterlite_bedrock::embedding_jobs::transport::{
    StatusReply, StatusTransport, StatusWireRequest,
};
use betterlite_bedrock::signing::executor::AwsSigningExecutor;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::Builder;

use super::Failure;
use super::env::{NoPorts, with_logging};
use super::input::Call;
use betterlite_secret_managers::secret_lookup::ports::OidcTokenCache;

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

impl CredentialSource for Source {
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

fn fetch_status(
    call: &Call,
    parameters: &mut BTreeMap<String, Box<RawValue>>,
    services: &StatusServices<'_, Source, Factory>,
    log: Option<&mut StatusLogger<'_>>,
) -> Result<Result<Json, EmbeddingJobsError>, Failure> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Failure::from)?;
    let now = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(CLOCK_SECONDS))
        .ok_or(Failure::Fixture)?;
    let request = AsyncInvokeStatusCall {
        invocation_arn: &call.invocation_arn,
        region: &call.region,
        optional_params: parameters,
        environment: &call.environment,
        now,
    };
    Ok(runtime.block_on(get_async_invoke_status(request, services, log)))
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
    let secrets = SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    };
    execute_with_source(call, source, &secrets)
}

fn execute_with_source(
    call: &Call,
    source: Source,
    secrets: &SecretLookup,
) -> Result<Execution, Failure> {
    let region_worker = ManagedRegionWorker::new().map_err(Failure::from)?;
    let seen: Arc<Mutex<Vec<StatusWireRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let factory = Factory {
        status: call.status,
        body: call.body.clone(),
        seen: Arc::clone(&seen),
        calls: Mutex::new(Vec::new()),
    };
    let mut parameters = parameters(call)?;
    let signing = AwsSigningExecutor::default();
    let services = StatusServices {
        source: &source,
        secrets,
        region_worker: &region_worker,
        factory: &factory,
        signing: &signing,
    };
    let (result, snapshots) = with_logging(call.logging, |log| {
        fetch_status(call, &mut parameters, &services, log)
    })?;
    Ok(Execution {
        result: result?,
        requests: seen.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        factory_calls: factory
            .calls
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner),
        resolved_regions: source
            .regions
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner),
        snapshots,
    })
}
