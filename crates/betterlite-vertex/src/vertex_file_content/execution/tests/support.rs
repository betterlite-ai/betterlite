use std::collections::BTreeMap;
use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::gcs_client::gcs_storage_auth::GcsBucketBase;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use betterlite_logging::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use betterlite_logging::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use betterlite_logging::gcs_mock::request::GcsRequest;
use betterlite_openai::files::records::FileContentRequest;
use http::HeaderMap;
use http::header::AUTHORIZATION;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

pub(crate) const BUCKET: &str = "bucket";
pub(crate) const FILE_ID: &str = "gs://bucket/litellm-vertex-files/input.jsonl";
pub(crate) const SPACED_FILE_ID: &str = "gs://bucket/litellm-vertex-files/a%20b.jsonl";
pub(crate) const MODEL_FILE_ID: &str =
    "gs://bucket/litellm-vertex-files/publishers/google/models/gemini-2.5-flash/batch.jsonl";
pub(crate) const LEGACY_FILE_ID: &str = "gs://bucket/legacy/input.jsonl";
pub(crate) const LEGACY_STORAGE_URL: &str =
    "https://storage.googleapis.com/storage/v1/b/bucket/o/legacy%2Finput.jsonl?alt=media";
pub(crate) const STORAGE_URL: &str = "https://storage.googleapis.com/storage/v1/b/bucket/o/litellm-vertex-files%2Finput.jsonl?alt=media";
pub(crate) const SPACED_STORAGE_URL: &str = "https://storage.googleapis.com/storage/v1/b/bucket/o/litellm-vertex-files%2Fa%20b.jsonl?alt=media";
pub(crate) const MODEL_STORAGE_URL: &str = "https://storage.googleapis.com/storage/v1/b/bucket/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-2.5-flash%2Fbatch.jsonl?alt=media";
pub(crate) const IAM_KEY: &str = "IAM_AUTH";
pub(crate) const SERVICE_ACCOUNT_PATH: &str = "/secrets/base-sa.json";

// what the read did, in order: an authorization with its credential, then one GET
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Authorized(Option<String>),
    Sent(String),
}

#[must_use]
pub(crate) fn token_for(credential: Option<&str>) -> String {
    match credential {
        Some(text) => format!("token-for-{text}"),
        None => format!("token-for-{IAM_KEY}"),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the test authentication failed")]
pub(crate) struct AuthFailure;

#[derive(Debug, thiserror::Error)]
#[error("the test transport failed")]
pub(crate) struct TransportFailure;

#[derive(Clone, Debug)]
pub(crate) struct FakeAuth {
    steps: Arc<Mutex<Vec<Step>>>,
    hang: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
}

impl FakeAuth {
    fn credential(request: &EnsureAccessToken) -> Option<String> {
        request
            .credentials
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned())
    }

    fn record(&self, credential: Option<String>) {
        self.steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Step::Authorized(credential));
    }
}

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = AuthFailure;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthFailure> {
        let credential = Self::credential(&request);
        self.record(credential.clone());
        if self.hang.load(Ordering::SeqCst) {
            pending::<()>().await;
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(AuthFailure);
        }
        Ok(AccessToken {
            token: SecretString::from(token_for(credential.as_deref())),
            project_id: "project".to_owned(),
        })
    }

    fn ensure_access_token(&self, _request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        panic!("the read path authorizes asynchronously");
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthFailure> {
        let Some(header) = request.auth_header else {
            return Err(AuthFailure);
        };
        Ok(TokenAndUrl {
            token: header,
            url: "https://storage.googleapis.com".to_owned(),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordingClient {
    steps: Arc<Mutex<Vec<Step>>>,
    sent: Arc<Mutex<Vec<GcsRequest>>>,
    status: u16,
    body: Vec<u8>,
    fail: Arc<AtomicBool>,
}

#[async_trait]
impl GcsHttpClient for RecordingClient {
    type Error = TransportFailure;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, TransportFailure> {
        let url = request.url().to_owned();
        self.steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Step::Sent(url.clone()));
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if self.fail.load(Ordering::SeqCst) {
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

pub(crate) struct Harness {
    pub(crate) auth: FakeAuth,
    pub(crate) client: RecordingClient,
    environment: BTreeMap<String, String>,
}

impl Harness {
    pub(crate) fn with_env(pairs: &[(&str, &str)]) -> Self {
        let mut environment = BTreeMap::new();
        for (name, value) in pairs {
            environment.insert((*name).to_owned(), (*value).to_owned());
        }
        let shared: Arc<Mutex<Vec<Step>>> = Arc::default();
        Self {
            auth: FakeAuth {
                steps: Arc::clone(&shared),
                hang: Arc::default(),
                fail: Arc::default(),
            },
            client: RecordingClient {
                steps: shared,
                sent: Arc::default(),
                status: 200,
                body: b"{}".to_vec(),
                fail: Arc::default(),
            },
            environment,
        }
    }

    pub(crate) fn set_auth_failure(&self, failed: bool) {
        self.auth.fail.store(failed, Ordering::SeqCst);
    }

    pub(crate) fn set_auth_hang(&self, hanging: bool) {
        self.auth.hang.store(hanging, Ordering::SeqCst);
    }

    pub(crate) fn set_transport_failure(&self, failed: bool) {
        self.client.fail.store(failed, Ordering::SeqCst);
    }

    pub(crate) fn responding(mut self, status: u16, body: &[u8]) -> Self {
        self.client.status = status;
        self.client.body = body.to_vec();
        self
    }

    pub(crate) fn base(&self) -> GcsBucketBase<FakeAuth, RecordingClient> {
        let source: Arc<dyn EnvironmentSource> = Arc::new(self.environment.clone());
        initialize(source, None, self.auth.clone(), self.client.clone())
            .expect("the test environment initializes the base")
    }

    pub(crate) fn steps(&self) -> Vec<Step> {
        self.auth
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn requests(&self) -> Vec<GcsRequest> {
        self.client
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn authorization(&self, position: usize) -> String {
        let requests = self.requests();
        let GcsRequest::Get(read) = &requests[position] else {
            panic!("the read only sends GET requests");
        };
        read.headers
            .get(AUTHORIZATION)
            .expect("the download carries the bearer token")
            .to_str()
            .expect("the bearer token is text")
            .to_owned()
    }
}

pub(crate) fn content_request(file_id: &str) -> FileContentRequest {
    FileContentRequest {
        file_id: file_id.to_owned(),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}
