use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::batch_call::transport::{
    ClientConfig, ControlPlaneFactory, ControlPlaneReply, ControlPlaneRequest,
    ControlPlaneTransport,
};
use betterlite_bedrock::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, Timeout, TransferError,
};
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::embedding_jobs::transport::{
    StatusReply, StatusTransport, StatusWireRequest,
};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::ReplyInput;

pub(super) struct Source {
    credentials: Credentials,
    regions: Mutex<Vec<String>>,
}

impl Source {
    pub(super) const fn new(credentials: Credentials) -> Self {
        Self {
            credentials,
            regions: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn regions(&self) -> Vec<String> {
        self.regions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
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

struct ClientRecord {
    service: String,
    region: String,
    access_key_id: String,
    session_token: Option<String>,
    signature_version: String,
}

struct PlaneState {
    configs: Vec<ClientRecord>,
    requests: Vec<ControlPlaneRequest>,
    replies: VecDeque<ReplyInput>,
}

pub(super) struct PlaneFactory {
    state: Arc<Mutex<PlaneState>>,
}

impl PlaneFactory {
    pub(super) fn new(replies: Vec<ReplyInput>) -> Self {
        let state = PlaneState {
            configs: Vec::new(),
            requests: Vec::new(),
            replies: VecDeque::from(replies),
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    // one client reaches one request per control plane call, so the two logs pair up
    pub(super) fn calls(&self, batch_id: &str) -> Value {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let calls: Vec<Value> = state
            .requests
            .iter()
            .zip(state.configs.iter())
            .map(|(request, config)| {
                let operation = if request.method == "POST" {
                    "stop_model_invocation_job"
                } else {
                    "get_model_invocation_job"
                };
                json!({
                    "service": &config.service,
                    "operation": operation,
                    "jobIdentifier": batch_id,
                    "region": &config.region,
                    "access_key_id": &config.access_key_id,
                    "session_token": &config.session_token,
                    "signature_version": &config.signature_version,
                })
            })
            .collect();
        drop(state);
        json!(calls)
    }
}

impl ControlPlaneFactory for PlaneFactory {
    type Client = PlaneClient;

    fn client(&self, config: ClientConfig<'_>) -> Self::Client {
        let record = ClientRecord {
            service: config.service_name.to_owned(),
            region: config.region.to_owned(),
            access_key_id: config.credentials.access_key_id.expose_secret().to_owned(),
            session_token: config
                .credentials
                .session_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            signature_version: config.signature_version.to_owned(),
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.configs.push(record);
        drop(state);
        PlaneClient {
            state: Arc::clone(&self.state),
        }
    }
}

pub(super) struct PlaneClient {
    state: Arc<Mutex<PlaneState>>,
}

impl ControlPlaneTransport for PlaneClient {
    fn send(
        &self,
        request: ControlPlaneRequest,
    ) -> impl Future<Output = Result<ControlPlaneReply, TransferError>> + Send {
        let queued = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.requests.push(request);
            state.replies.pop_front()
        };
        let outcome = match queued {
            Some(reply) => Ok(ControlPlaneReply {
                status: reply.status,
                headers: reply.headers.into_iter().collect(),
                body: reply.body.into_bytes(),
            }),
            None => Err(TransferError::Timeout {
                elapsed: Duration::ZERO,
            }),
        };
        ready(outcome)
    }
}

struct AsyncState {
    reply: ReplyInput,
    urls: Vec<String>,
}

impl AsyncState {
    fn new(reply: &ReplyInput) -> Self {
        Self {
            reply: reply.clone(),
            urls: Vec::new(),
        }
    }
}

pub(super) struct AsyncFixture {
    state: Arc<Mutex<AsyncState>>,
}

impl AsyncFixture {
    pub(super) fn new(reply: &ReplyInput) -> Self {
        Self {
            state: Arc::new(Mutex::new(AsyncState::new(reply))),
        }
    }

    pub(super) fn urls(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .urls
            .clone()
    }
}

impl ClientFactory for AsyncFixture {
    type Client = AsyncClient;

    fn sync_client(&self, _timeout: Option<Timeout>) -> Self::Client {
        AsyncClient {
            state: Arc::clone(&self.state),
        }
    }

    fn async_client(&self, _provider: &str, _timeout: Option<Timeout>) -> Self::Client {
        AsyncClient {
            state: Arc::clone(&self.state),
        }
    }
}

pub(super) struct AsyncClient {
    state: Arc<Mutex<AsyncState>>,
}

impl StatusTransport for AsyncClient {
    fn get(
        &self,
        request: StatusWireRequest,
    ) -> impl Future<Output = Result<StatusReply, TransferError>> + Send {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.urls.push(request.url);
        let status = state.reply.status;
        let body = state.reply.body.clone().into_bytes();
        drop(state);
        ready(Ok(StatusReply { status, body }))
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
