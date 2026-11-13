use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use secrecy::SecretString;

use crate::fields::payload::StandardLoggingPayload;
use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use crate::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use crate::gcs_client::gcs_storage_auth::{GcsBucketBase, RequestHeaders};
use crate::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth, VertexAuthProvider,
};
use crate::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use crate::gcs_mock::request::{GcsReadRequest, GcsRequest};

pub(crate) const TOKEN: &str = "original-token";
pub(crate) const PROJECT: &str = "original-project";

#[derive(Debug, thiserror::Error)]
#[error("the test authentication failed")]
pub(crate) struct AuthFailure;

#[derive(Debug, thiserror::Error)]
#[error("the test transport failed")]
pub(crate) struct TransportFailure;

#[derive(Clone, Debug)]
pub(crate) struct FakeAuth {
    pub(crate) token: String,
    pub(crate) project: String,
    pub(crate) fail: bool,
    pub(crate) async_calls: Arc<AtomicUsize>,
    pub(crate) sync_calls: Arc<AtomicUsize>,
    pub(crate) ensure_requests: Arc<Mutex<Vec<EnsureAccessToken>>>,
    pub(crate) token_requests: Arc<Mutex<Vec<GetTokenAndUrl>>>,
}

impl FakeAuth {
    pub(crate) fn new() -> Self {
        Self {
            token: TOKEN.to_owned(),
            project: PROJECT.to_owned(),
            fail: false,
            async_calls: Arc::new(AtomicUsize::new(0)),
            sync_calls: Arc::new(AtomicUsize::new(0)),
            ensure_requests: Arc::new(Mutex::new(Vec::new())),
            token_requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn minting(token: &str) -> Self {
        let mut auth = Self::new();
        auth.token = token.to_owned();
        auth
    }

    pub(crate) fn failing() -> Self {
        let mut auth = Self::new();
        auth.fail = true;
        auth
    }

    pub(crate) fn last_ensure(&self) -> EnsureAccessToken {
        self.ensure_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .expect("the authentication was asked for a token")
            .clone()
    }

    pub(crate) fn ensure_count(&self) -> usize {
        self.ensure_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn last_token_request(&self) -> GetTokenAndUrl {
        self.token_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .expect("the authentication was asked for a token and a URL")
            .clone()
    }
}

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = AuthFailure;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthFailure> {
        self.async_calls.fetch_add(1, Ordering::SeqCst);
        self.ensure_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if self.fail {
            return Err(AuthFailure);
        }
        Ok(AccessToken {
            token: SecretString::from(self.token.clone()),
            project_id: self.project.clone(),
        })
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        self.sync_calls.fetch_add(1, Ordering::SeqCst);
        self.ensure_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if self.fail {
            return Err(AuthFailure);
        }
        Ok(AccessToken {
            token: SecretString::from(self.token.clone()),
            project_id: self.project.clone(),
        })
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthFailure> {
        self.token_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if self.fail {
            return Err(AuthFailure);
        }
        Ok(TokenAndUrl {
            token: SecretString::from(self.token.clone()),
            url: "https://original.example.com".to_owned(),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordingClient {
    pub(crate) sent: Arc<Mutex<Vec<GcsRequest>>>,
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) fail: bool,
}

impl RecordingClient {
    pub(crate) fn new() -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            status: 200,
            body: b"{}".to_vec(),
            fail: false,
        }
    }

    pub(crate) fn responding(status: u16, body: &str) -> Self {
        let mut client = Self::new();
        client.status = status;
        client.body = body.as_bytes().to_vec();
        client
    }

    pub(crate) fn failing() -> Self {
        let mut client = Self::new();
        client.fail = true;
        client
    }
}

#[async_trait]
impl GcsHttpClient for RecordingClient {
    type Error = TransportFailure;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, TransportFailure> {
        let url = request.url().to_owned();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if self.fail {
            return Err(TransportFailure);
        }
        Ok(GcsHttpResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
            url,
            elapsed: Duration::ZERO,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FakeEnvironment {
    values: Arc<Mutex<BTreeMap<String, String>>>,
}

impl FakeEnvironment {
    pub(crate) fn new(pairs: &[(&str, &str)]) -> Self {
        let mut values = BTreeMap::new();
        for (name, value) in pairs {
            values.insert((*name).to_owned(), (*value).to_owned());
        }
        Self {
            values: Arc::new(Mutex::new(values)),
        }
    }

    pub(crate) fn set(&self, name: &str, value: &str) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value.to_owned());
    }

    pub(crate) fn remove(&self, name: &str) {
        let previous = self
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
        assert!(previous.is_some(), "the removed variable was set");
    }
}

impl EnvironmentSource for FakeEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Harness {
    pub(crate) environment: FakeEnvironment,
    pub(crate) auth: FakeAuth,
    pub(crate) client: RecordingClient,
}

impl Harness {
    pub(crate) fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            environment: FakeEnvironment::new(pairs),
            auth: FakeAuth::new(),
            client: RecordingClient::new(),
        }
    }

    pub(crate) fn build(
        &self,
        bucket_name: Option<&str>,
    ) -> GcsBucketBase<FakeAuth, RecordingClient> {
        build_with(
            &self.environment,
            bucket_name,
            self.auth.clone(),
            self.client.clone(),
        )
    }

    pub(crate) fn requests(&self) -> Vec<GcsRequest> {
        self.client
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn last_request(&self) -> GcsRequest {
        self.requests()
            .pop()
            .expect("the client was sent a request")
    }
}

pub(crate) fn build_with(
    environment: &FakeEnvironment,
    bucket_name: Option<&str>,
    auth: FakeAuth,
    client: RecordingClient,
) -> GcsBucketBase<FakeAuth, RecordingClient> {
    let source: Arc<dyn EnvironmentSource> = Arc::new(environment.clone());
    initialize(source, bucket_name.map(str::to_owned), auth, client)
        .expect("the test environment initializes the base")
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> RequestHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

pub(crate) fn ensure_request() -> EnsureAccessToken {
    EnsureAccessToken {
        credentials: None,
        project_id: None,
        custom_llm_provider: VertexAuthProvider::VertexAi,
    }
}

pub(crate) fn get(url: &str) -> GcsRequest {
    GcsRequest::Get(GcsReadRequest {
        url: url.to_owned(),
        params: BTreeMap::new(),
        headers: HeaderMap::new(),
        follow_redirects: None,
    })
}

pub(crate) fn logging_payload() -> StandardLoggingPayload {
    serde_json::from_str(LOGGING_PAYLOAD).expect("the fixture is a standard logging payload")
}

const LOGGING_PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "stream": false,
    "response_cost": 0.5,
    "cost_breakdown": null,
    "autorouter_savings_estimate": null,
    "autorouter_baseline_observation": null,
    "response_cost_failure_debug_info": null,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "custom_llm_provider": "openai",
    "total_tokens": 10,
    "prompt_tokens": 4,
    "completion_tokens": 6,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "model_id": null,
    "model_group": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "cache_hit": null,
    "cache_key": null,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "end_user": null,
    "requester_ip_address": null,
    "user_agent": null,
    "messages": null,
    "response": null,
    "error_str": null,
    "error_information": null,
    "model_parameters": {},
    "hidden_params": {},
    "guardrail_information": null,
    "standard_built_in_tools_params": null
}"#;
