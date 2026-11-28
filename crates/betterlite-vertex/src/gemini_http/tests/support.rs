#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_prompts::arguments::json::Object;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;

use crate::credential_sources::sdk::CredentialClock;
use crate::gemini_http::call::{Clients, GeminiCall, SuppliedClient, VertexLlm};
use crate::gemini_http::post::{
    Chunks, GeminiPost, OpenFuture, OpenedReply, PostFuture, PostReply, PostRequest,
};
use crate::gemini_response_parts::projection::CallMode;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;
use crate::token_refresh::CredentialRefresh;
use crate::vertex_embedding_call::post::{ClientTimeout, PostError};
use betterlite_core::api::vector_store_http_write::service::ClientKind;

pub(super) const TOKEN: &str = "ya29.vertex-test-token";
pub(super) const GEMINI_KEY: &str = "gemini-test-key";
pub(super) const PROJECT: &str = "project-p";
pub(super) const LOCATION: &str = "us-central1";
pub(super) const MODEL: &str = "gemini-2.5-flash";
pub(super) const BODY: &str = r#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#;
pub(super) const ANSWER: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}}"#;
pub(super) const STREAM_CHUNK: &str =
    r#"data: {"responseId":"r1","candidates":[{"content":{"parts":[{"text":"he"}]}}]}"#;
pub(super) const SCHEMA_ERROR: &str = "GenerateContentRequest.tools[0].function_declarations[0].parameters.properties: should be non-empty for OBJECT type";

pub(super) static NO_HEADERS: BTreeMap<String, String> = BTreeMap::new();
pub(super) static NO_SECRET_HEADERS: BTreeMap<String, SecretString> = BTreeMap::new();

pub(super) fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

pub(super) fn exposed(headers: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

#[derive(Clone, Debug)]
pub(super) enum Scripted<Body> {
    Status {
        status: u16,
        headers: HeaderMap,
        body: Body,
    },
    Timeout,
    Failed,
}

pub(super) type Reply = Scripted<Vec<u8>>;
pub(super) type OpenReply = Scripted<Vec<Piece>>;

#[derive(Debug)]
pub(super) enum Piece {
    Bytes(Vec<u8>),
    Failed(HttpError),
}

pub(super) fn complete_reply(status: u16, body: &str) -> Reply {
    Reply::Status {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn error_reply(status: u16, pairs: &[(&str, &str)], body: &str) -> Reply {
    Reply::Status {
        status,
        headers: headers(pairs),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn stream_reply(status: u16, pieces: Vec<Piece>) -> OpenReply {
    OpenReply::Status {
        status,
        headers: HeaderMap::new(),
        body: pieces,
    }
}

pub(super) fn line(text: &str) -> Piece {
    Piece::Bytes(format!("{text}\n\n").into_bytes())
}

#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
    pub(super) timeout: ClientTimeout,
}

pub(super) struct Recorder {
    posts: Mutex<Vec<Recorded>>,
    opens: Mutex<Vec<Recorded>>,
    post_replies: Mutex<VecDeque<Reply>>,
    open_replies: Mutex<VecDeque<OpenReply>>,
}

impl Recorder {
    pub(super) fn scripted(posts: Vec<Reply>, opens: Vec<OpenReply>) -> Self {
        Self {
            posts: Mutex::new(Vec::new()),
            opens: Mutex::new(Vec::new()),
            post_replies: Mutex::new(VecDeque::from(posts)),
            open_replies: Mutex::new(VecDeque::from(opens)),
        }
    }

    pub(super) fn posts(&self) -> Vec<Recorded> {
        let posts = self.posts.lock().unwrap_or_else(PoisonError::into_inner);
        posts.clone()
    }

    pub(super) fn opens(&self) -> Vec<Recorded> {
        let opens = self.opens.lock().unwrap_or_else(PoisonError::into_inner);
        opens.clone()
    }

    fn take<Body>(
        requests: &Mutex<Vec<Recorded>>,
        replies: &Mutex<VecDeque<Body>>,
        request: &PostRequest<'_>,
    ) -> Body {
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(request));
        replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test scripted a reply")
    }
}

fn recorded(request: &PostRequest<'_>) -> Recorded {
    Recorded {
        url: request.url.to_owned(),
        headers: exposed(request.headers),
        body: request.body.to_vec(),
        timeout: request.timeout,
    }
}

impl GeminiPost for Recorder {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request> {
        let reply = Self::take(&self.posts, &self.post_replies, &request);
        Box::pin(async move { complete(reply) })
    }

    fn open<'request>(&'request self, request: PostRequest<'request>) -> OpenFuture<'request> {
        let reply = Self::take(&self.opens, &self.open_replies, &request);
        Box::pin(async move { open(reply) })
    }
}

fn answer<Body>(reply: Scripted<Body>) -> Result<(u16, HeaderMap, Body), PostError> {
    match reply {
        Scripted::Status {
            status,
            headers,
            body,
        } => Ok((status, headers, body)),
        Scripted::Timeout => Err(PostError::Timeout),
        Scripted::Failed => Err(PostError::Send {
            source: TransportError::Closed,
        }),
    }
}

fn complete(reply: Reply) -> Result<PostReply, PostError> {
    let (status, headers, body) = answer(reply)?;
    Ok(PostReply {
        status,
        headers,
        body,
    })
}

fn open(reply: OpenReply) -> Result<OpenedReply, PostError> {
    let (status, headers, pieces) = answer(reply)?;
    Ok(OpenedReply {
        status,
        headers,
        body: chunks(pieces),
    })
}

fn chunks(pieces: Vec<Piece>) -> Chunks {
    stream::iter(pieces.into_iter().map(|piece| match piece {
        Piece::Bytes(bytes) => Ok(bytes),
        Piece::Failed(source) => Err(source),
    }))
    .boxed()
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

pub(super) fn llm(auth: &Auth) -> VertexLlm {
    VertexLlm {
        cache: Arc::clone(&auth.cache),
        refresh: Arc::clone(&auth.refresh),
    }
}

pub(super) fn pooled(recorder: &Arc<Recorder>) -> Clients {
    let client = Arc::clone(recorder);
    Clients {
        supplied: None,
        pooled: client,
    }
}

pub(super) fn supplied(
    recorder: &Arc<Recorder>,
    kind: ClientKind,
    pooled: &Arc<Recorder>,
) -> Clients {
    let transport = Arc::clone(recorder);
    let client = Arc::clone(pooled);
    Clients {
        supplied: Some(SuppliedClient { kind, transport }),
        pooled: client,
    }
}

pub(super) fn vertex_call<'call>(
    body: &'call Object,
    optional: &'call Object,
) -> GeminiCall<'call> {
    GeminiCall {
        provider: VertexCapabilityProvider::VertexAi,
        model: MODEL,
        body,
        optional_params: optional,
        api_base: None,
        gemini_api_key: None,
        project: Some(PROJECT),
        location: Some(LOCATION),
        credentials: None,
        extra_headers: &NO_HEADERS,
        timeout: None,
        now: 1_700_000_000,
        call_mode: CallMode::Tools,
    }
}
