#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::EmbeddingInput;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;

use crate::credential_sources::sdk::CredentialClock;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;
use crate::token_refresh::CredentialRefresh;
use crate::vertex_embedding_call::multimodal::{DEFAULT_TIMEOUT, MultimodalCall};
use crate::vertex_embedding_call::post::{
    ClientTimeout, EmbeddingPost, PostError, PostFuture, PostReply, PostRequest,
};
use crate::vertex_embedding_call::text::{TextAuth, TextCall};

pub(super) const TOKEN: &str = "ya29.vertex-test-token";
pub(super) const GEMINI_KEY: &str = "gemini-test-key";
pub(super) const PROJECT: &str = "project-p";
pub(super) const LOCATION: &str = "us-central1";
pub(super) const TEXT_MODEL: &str = "text-embedding-005";
pub(super) const MULTIMODAL_MODEL: &str = "multimodalembedding@001";

pub(super) const TEXT_ANSWER: &str = r#"{"predictions":[{"embeddings":{"values":[0.1,0.2],"statistics":{"token_count":2,"truncated":false}}}]}"#;
pub(super) const MULTIMODAL_ANSWER: &str = r#"{"predictions":[{"textEmbedding":[0.5,0.25]}]}"#;

pub(super) static NO_HEADERS: BTreeMap<String, String> = BTreeMap::new();

pub(super) fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn extra(text: &str) -> Extra {
    let Json::Object(fields) = json(text) else {
        panic!("the test fixture is not a JSON object");
    };
    fields
}

pub(super) fn reply(status: u16, body: &str) -> Reply {
    let body = body.as_bytes().to_vec();
    Reply::Status { status, body }
}

#[derive(Clone, Debug)]
pub(super) enum Reply {
    Status { status: u16, body: Vec<u8> },
    Timeout,
    Failed,
}

#[derive(Clone, Debug)]
pub(super) struct RecordedPost {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
    pub(super) timeout: ClientTimeout,
}

pub(super) struct PostRecorder {
    calls: Mutex<Vec<RecordedPost>>,
    replies: Mutex<VecDeque<Reply>>,
}

impl PostRecorder {
    pub(super) fn scripted(replies: Vec<Reply>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            replies: Mutex::new(VecDeque::from(replies)),
        }
    }

    pub(super) fn recorded(&self) -> Vec<RecordedPost> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl EmbeddingPost for PostRecorder {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request> {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        let recorded = RecordedPost {
            url: request.url.to_owned(),
            headers,
            body: request.body.to_vec(),
            timeout: request.timeout,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test scripted an embedding reply");
        Box::pin(async move { answer(reply) })
    }
}

fn answer(reply: Reply) -> Result<PostReply, PostError> {
    match reply {
        Reply::Status { status, body } => Ok(PostReply { status, body }),
        Reply::Timeout => Err(PostError::Timeout),
        Reply::Failed => Err(PostError::Send {
            source: TransportError::Closed,
        }),
    }
}

pub(super) struct PendingPost {
    pub(super) dropped: Arc<AtomicBool>,
}

impl EmbeddingPost for PendingPost {
    fn post<'request>(&'request self, _request: PostRequest<'request>) -> PostFuture<'request> {
        let dropped = Arc::clone(&self.dropped);
        Box::pin(async move {
            let _signal = DropSignal { dropped };
            pending::<()>().await;
            Err(PostError::Timeout)
        })
    }
}

struct DropSignal {
    dropped: Arc<AtomicBool>,
}

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

pub(super) struct FixedClock(pub(super) SystemTime);

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

pub(super) struct FixedExecutor {
    pub(super) loads: AtomicUsize,
    token: SecretString,
    project: String,
}

impl CredentialExecutor for FixedExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.loads.fetch_add(1, Ordering::SeqCst);
        let credential = Arc::new(CredentialSnapshot {
            token: Some(self.token.clone()),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some(self.project.clone()),
        });
        let project = Some(self.project.clone());
        Box::pin(async move {
            Ok(LoadedCredential {
                credential: Some(credential),
                project,
            })
        })
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(async { Err(CredentialCacheError::MissingToken) })
    }
}

pub(super) struct Auth {
    pub(super) cache: Arc<CredentialCache>,
    pub(super) refresh: Arc<CredentialRefresh>,
    pub(super) executor: Arc<FixedExecutor>,
}

pub(super) fn auth() -> Auth {
    let executor = Arc::new(FixedExecutor {
        loads: AtomicUsize::new(0),
        token: SecretString::from(TOKEN.to_owned()),
        project: PROJECT.to_owned(),
    });
    let handle = Arc::clone(&executor);
    let credential_executor: Arc<dyn CredentialExecutor> = handle;
    let cache = Arc::new(CredentialCache::new(
        Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
        credential_executor,
    ));
    let refresh = Arc::new(CredentialRefresh::new(
        Arc::clone(&cache),
        TaskTracker::new(),
    ));
    Auth {
        cache,
        refresh,
        executor,
    }
}

pub(super) fn text_auth(auth: &Auth) -> TextAuth<'_> {
    TextAuth {
        cache: auth.cache.as_ref(),
        refresh: auth.refresh.as_ref(),
    }
}

pub(super) const fn text_call<'call>(
    input: &'call EmbeddingInput,
    optional_params: &'call Json,
) -> TextCall<'call> {
    TextCall {
        provider: VertexCapabilityProvider::VertexAi,
        model: TEXT_MODEL,
        input,
        optional_params,
        litellm_params: None,
        api_base: None,
        gemini_api_key: None,
        project: Some(PROJECT),
        location: Some(LOCATION),
        credentials: None,
        extra_headers: &NO_HEADERS,
        timeout: None,
        aembedding: false,
    }
}

pub(super) const fn multimodal_call<'call>(
    model: &'call str,
    input: &'call Json,
    optional_params: &'call Extra,
    headers: &'call BTreeMap<String, String>,
) -> MultimodalCall<'call> {
    MultimodalCall {
        provider: VertexCapabilityProvider::VertexAi,
        model,
        input,
        optional_params,
        headers,
        api_base: None,
        gemini_api_key: None,
        project: Some(PROJECT),
        location: Some(LOCATION),
        credentials: None,
        timeout: Some(DEFAULT_TIMEOUT),
        aembedding: false,
    }
}
