use std::collections::VecDeque;
use std::fs;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;
use tokio::task;

use crate::workload_identity::auth::MonotonicClock;
use crate::workload_identity::cache::WorkloadIdentityCredentials;
use crate::workload_identity::config::{OpenAiWorkloadIdentityConfig, SdkCapability};
use crate::workload_identity::error::WorkloadIdentityError;
use crate::workload_identity::transport::WorkloadIdentityTransport;

#[derive(Clone)]
pub(super) enum Reply {
    Failure,
    Answer { status: u16, body: String },
}

#[derive(Debug, PartialEq)]
pub(super) struct SentRequest {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) json: bool,
    pub(super) body: String,
    pub(super) deadline: Duration,
}

#[derive(Default)]
pub(super) struct Script {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<SentRequest>>,
}

impl Script {
    pub(super) fn replying(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn pending(&self) -> usize {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn sent(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn take(&self) -> Vec<SentRequest> {
        mem::take(&mut *self.requests.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn answer(&self, request: &OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let (json, body) = match &request.body {
            RequestBody::Json(data) => (true, String::from_utf8(data.clone()).unwrap_or_default()),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                (false, String::new())
            }
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SentRequest {
                method: request.method.as_str().to_owned(),
                url: request.url.to_string(),
                json,
                body,
                deadline: request.deadline,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(Reply::Answer { status, body: text }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: text.into_bytes(),
            }),
            Some(Reply::Failure) | None => Err(TransportError::Closed),
        }
    }
}

impl WorkloadIdentityTransport for Script {
    // yields once so a second caller is polled while the first is mid-exchange
    async fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        task::yield_now().await;
        self.answer(&request)
    }
}

#[derive(Clone, Default)]
pub(super) struct Clock(Arc<Mutex<Duration>>);

impl Clock {
    pub(super) fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(by);
    }

    pub(super) fn monotonic(&self) -> MonotonicClock {
        let shared = Arc::clone(&self.0);
        Arc::new(move || *shared.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

pub(super) fn answer(status: u16, body: &str) -> Reply {
    Reply::Answer {
        status,
        body: body.to_owned(),
    }
}

pub(super) fn token_reply(token: &str, expires_in: &str) -> Reply {
    answer(
        200,
        &format!(r#"{{"access_token":"{token}","expires_in":{expires_in}}}"#),
    )
}

pub(super) fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

pub(super) fn millis(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

pub(super) struct Fixture {
    pub(super) credentials: WorkloadIdentityCredentials<Script>,
    pub(super) script: Arc<Script>,
    pub(super) clock: Clock,
    pub(super) dir: TempDir,
}

impl Fixture {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self::with_sdk(replies, SdkCapability::AVAILABLE)
    }

    pub(super) fn with_sdk(replies: Vec<Reply>, sdk: SdkCapability) -> Self {
        let script = Script::replying(replies);
        let clock = Clock::default();
        let credentials =
            WorkloadIdentityCredentials::new(Arc::clone(&script), clock.monotonic(), sdk);
        Self {
            credentials,
            script,
            clock,
            dir: TempDir::new().unwrap(),
        }
    }

    pub(super) fn config(&self, name: &str, token: &str) -> OpenAiWorkloadIdentityConfig {
        let path = self.dir.path().join(name);
        fs::write(&path, token).unwrap();
        OpenAiWorkloadIdentityConfig {
            identity_provider_id: format!("idp-{name}"),
            service_account_id: format!("sa-{name}"),
            token_file: path.to_str().unwrap().to_owned(),
        }
    }

    pub(super) async fn bearer(
        &self,
        config: &OpenAiWorkloadIdentityConfig,
    ) -> Result<SecretString, WorkloadIdentityError> {
        self.credentials
            .workload_identity_auth(config)?
            .get_token()
            .await
    }

    pub(super) async fn outcome(&self, config: &OpenAiWorkloadIdentityConfig) -> String {
        match self.bearer(config).await {
            Ok(token) => token.expose_secret().to_owned(),
            Err(failure) => failure.to_string(),
        }
    }

    pub(super) async fn token(&self, config: &OpenAiWorkloadIdentityConfig) -> String {
        self.bearer(config)
            .await
            .unwrap()
            .expose_secret()
            .to_owned()
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}
