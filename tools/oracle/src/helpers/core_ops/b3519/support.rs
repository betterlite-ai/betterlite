use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_provider::e2b::{
    E2bError, SandboxHttp as E2bHttp, SandboxRequest as E2bRequest, SandboxStream as E2bStream,
};
use betterlite_provider::opensandbox::helpers::{
    OpenSandboxError, PollClock, SandboxHttp as OpenHttp, SandboxRequest as OpenRequest,
    SandboxStream as OpenStream,
};
use betterlite_provider::sandbox_dispatch::request::SandboxContainer;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::Method;
use http::header::HeaderMap;
use secrecy::SecretString;
use serde::Serialize;
use url::Url;

use super::input::Reply;

pub(crate) type Environment = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) json: Option<serde_json::Value>,
}

#[derive(Clone)]
struct StoredReply {
    status: u16,
    body: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct FakeHttp {
    replies: Arc<Mutex<VecDeque<StoredReply>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeHttp {
    pub(crate) fn new(replies: &[Reply]) -> Self {
        let replies = replies.iter().map(|reply| StoredReply {
            status: reply.status.unwrap_or(200),
            body: reply.body.clone().unwrap_or_default().into_bytes(),
        });
        Self {
            replies: Arc::new(Mutex::new(replies.collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn exchange(
        &self,
        method: &Method,
        url: &Url,
        headers: &HeaderMap,
        body: Vec<u8>,
    ) -> StoredReply {
        let headers = headers
            .iter()
            .map(|(name, value)| {
                let text = value
                    .to_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|error| format!("<{error}>"));
                (name.as_str().to_owned(), text)
            })
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                method: method.as_str().to_owned(),
                url: url.to_string(),
                headers,
                json: serde_json::from_slice(&body).ok(),
            });
        self.next_reply()
    }

    fn next_reply(&self) -> StoredReply {
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(reply) = replies.pop_front() else {
            return StoredReply {
                status: 200,
                body: Vec::new(),
            };
        };
        if replies.is_empty() {
            replies.push_back(reply.clone());
        }
        reply
    }
}

fn reply_stream<Error>(
    reply: StoredReply,
) -> futures_util::stream::BoxStream<'static, Result<Vec<u8>, Error>>
where
    Error: Send + 'static,
{
    stream::iter(vec![Ok::<Vec<u8>, Error>(reply.body)]).boxed()
}

#[async_trait]
impl E2bHttp for FakeHttp {
    async fn send(&self, request: E2bRequest) -> Result<E2bStream, E2bError> {
        let reply = self.exchange(
            &request.method,
            &request.url,
            &request.headers,
            request.body,
        );
        Ok(E2bStream {
            status: reply.status,
            body: reply_stream(reply),
        })
    }
}

#[async_trait]
impl OpenHttp for FakeHttp {
    async fn send(&self, request: OpenRequest) -> Result<OpenStream, OpenSandboxError> {
        let reply = self.exchange(
            &request.method,
            &request.url,
            &request.headers,
            request.body,
        );
        Ok(OpenStream {
            status: reply.status,
            body: reply_stream(reply),
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct StepClock {
    now_millis: Arc<AtomicU64>,
}

#[async_trait]
impl PollClock for StepClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.now_millis.load(Ordering::Relaxed))
    }

    async fn sleep(&self, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.now_millis.fetch_add(millis, Ordering::Relaxed);
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no network in the oracle".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(crate) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(crate) fn container_value(container: &SandboxContainer) -> serde_json::Value {
    match container {
        SandboxContainer::E2b(handle) => serde_json::json!({
            "id": handle.id,
            "provider": handle.provider,
            "domain": handle.domain,
        }),
        SandboxContainer::OpenSandbox(handle) => serde_json::json!({
            "id": handle.container.id,
            "provider": handle.container.provider,
            "domain": handle.container.domain,
        }),
    }
}
