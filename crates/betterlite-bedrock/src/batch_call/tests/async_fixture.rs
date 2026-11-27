use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::batches::records::BatchObject;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

use crate::batch_call::async_status::{AsyncStatusCall, retrieve_async_status};
use crate::batch_call::error::BatchCallError;
use crate::bedrock_embedding_io::credentials::ManagedRegionWorker;
use crate::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, Timeout, TransferError,
};
use crate::embedding_jobs::status::StatusServices;
use crate::embedding_jobs::transport::{StatusReply, StatusTransport, StatusWireRequest};
use crate::signing::executor::AwsSigningExecutor;

use super::fixture::{Source, runtime};

pub(super) struct AsyncFactory {
    requests: Arc<Mutex<Vec<StatusWireRequest>>>,
    status: u16,
    body: Vec<u8>,
}

impl AsyncFactory {
    pub(super) fn new(status: u16, body: &str) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            status,
            body: body.as_bytes().to_vec(),
        }
    }

    pub(super) fn requests(&self) -> Vec<StatusWireRequest> {
        self.requests.lock().expect("the request lock").clone()
    }
}

struct AsyncClient {
    requests: Arc<Mutex<Vec<StatusWireRequest>>>,
    status: u16,
    body: Vec<u8>,
}

impl ClientFactory for AsyncFactory {
    type Client = AsyncClient;

    fn sync_client(&self, _timeout: Option<Timeout>) -> AsyncClient {
        panic!("the async status call never asks for the sync client")
    }

    fn async_client(&self, _provider: &str, _timeout: Option<Timeout>) -> AsyncClient {
        AsyncClient {
            requests: Arc::clone(&self.requests),
            status: self.status,
            body: self.body.clone(),
        }
    }
}

impl StatusTransport for AsyncClient {
    fn get(
        &self,
        request: StatusWireRequest,
    ) -> impl Future<Output = Result<StatusReply, TransferError>> + Send {
        self.requests
            .lock()
            .expect("the request lock")
            .push(request);
        ready(Ok(StatusReply {
            status: self.status,
            body: self.body.clone(),
        }))
    }
}

// B173's ClientFactory needs a post for the embedding calls this task does not make
impl EmbeddingTransport for AsyncClient {
    fn post(
        &self,
        _request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        ready(Err(TransferError::Failed(TransportError::Closed)))
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

fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn with_services<Output, Invoke>(
    source: &Source,
    factory: &AsyncFactory,
    invoke: Invoke,
) -> Output
where
    Invoke: FnOnce(&StatusServices<'_, Source, AsyncFactory>) -> Output,
{
    let secrets = secret_lookup();
    let region_worker = ManagedRegionWorker::new().expect("the region worker starts");
    let signing = AwsSigningExecutor::default();
    let services = StatusServices {
        source,
        secrets: &secrets,
        region_worker: &region_worker,
        factory,
        signing: &signing,
    };
    invoke(&services)
}

pub(super) fn retrieve(
    call: AsyncStatusCall<'_>,
    source: &Source,
    factory: &AsyncFactory,
) -> Result<BatchObject, BatchCallError> {
    with_services(source, factory, |services| {
        runtime().block_on(retrieve_async_status(call, services, None))
    })
}
