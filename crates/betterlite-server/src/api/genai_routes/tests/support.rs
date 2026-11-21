use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_core::api::genai_chat_output::GenAiOutputError;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_openai::chat::{ChatMessage, MessageContent, Role as ChatRole};
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use indexmap::IndexMap;
use tower::ServiceExt as _;

use crate::api::genai_routes::ports::{
    GenaiRoutesState, GenerateContentCall, GenerateContentDispatch, GenerationAnswer,
    GenerationBody, Modality, PromptTokenDetail, TokenCountRequest, TokenCountResponse,
    TokenCounter, TranslateContents,
};
use crate::api::genai_routes::routes;
use crate::auth::auth_identity::AuthContext;

pub(super) fn auth_context() -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: "admin-user".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: vec![Role::PlatformAdmin],
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn object(fields: Vec<(&str, Json)>) -> Object {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn message(role: ChatRole, content: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(MessageContent::Text(content.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

pub(super) fn unscripted() -> ProxyException {
    ProxyException {
        message: "no scripted outcome".to_owned(),
        ..failure("500", "internal_server_error", None)
    }
}

pub(super) fn failure(code: &str, error_type: &str, param: Option<&str>) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "the provider refused".to_owned(),
        error_type: error_type.to_owned(),
        param: param.map(str::to_owned),
        code: ExceptionCode::Text(code.to_owned()),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

pub(super) fn with_header(
    exception: ProxyException,
    name: &'static str,
    value: &str,
) -> ProxyException {
    let mut headers = exception.headers.clone();
    headers.insert(name.to_owned(), value.to_owned());
    ProxyException {
        headers,
        ..exception
    }
}

pub(super) fn answer(body: GenerationBody) -> GenerationAnswer {
    GenerationAnswer {
        headers: HeaderMap::new(),
        body,
    }
}

pub(super) fn events(
    chunks: Vec<Vec<u8>>,
) -> BoxStream<'static, Result<Vec<u8>, GenAiOutputError>> {
    stream::iter(chunks.into_iter().map(Ok)).boxed()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedCall {
    pub(super) model: String,
    pub(super) data: Object,
    pub(super) subject: String,
    pub(super) headers: HeaderMap,
}

pub(super) struct FakeGeneration {
    generate: Mutex<Option<Result<GenerationAnswer, Box<ProxyException>>>>,
    stream: Mutex<Option<Result<GenerationAnswer, Box<ProxyException>>>>,
    generate_calls: Mutex<Vec<RecordedCall>>,
    stream_calls: Mutex<Vec<RecordedCall>>,
}

impl FakeGeneration {
    fn new(
        generate: Option<Result<GenerationAnswer, Box<ProxyException>>>,
        stream: Option<Result<GenerationAnswer, Box<ProxyException>>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            generate: Mutex::new(generate),
            stream: Mutex::new(stream),
            generate_calls: Mutex::new(Vec::new()),
            stream_calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn silent() -> Arc<Self> {
        Self::new(None, None)
    }

    pub(super) fn answering(answer: GenerationAnswer) -> Arc<Self> {
        Self::new(Some(Ok(answer)), None)
    }

    pub(super) fn streaming(answer: GenerationAnswer) -> Arc<Self> {
        Self::new(None, Some(Ok(answer)))
    }

    pub(super) fn failing(exception: ProxyException) -> Arc<Self> {
        Self::new(Some(Err(Box::new(exception))), None)
    }

    pub(super) fn stream_failing(exception: ProxyException) -> Arc<Self> {
        Self::new(None, Some(Err(Box::new(exception))))
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.generate_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn streamed(&self) -> Vec<RecordedCall> {
        self.stream_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn recorded(call: &GenerateContentCall<'_>) -> RecordedCall {
    RecordedCall {
        model: call.model.to_owned(),
        data: call.data.clone(),
        subject: call.context.principal.subject.clone(),
        headers: call.headers.clone(),
    }
}

fn take(
    outcome: &Mutex<Option<Result<GenerationAnswer, Box<ProxyException>>>>,
) -> Result<GenerationAnswer, Box<ProxyException>> {
    outcome
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or_else(|| Err(Box::new(unscripted())))
}

fn record(recorded_calls: &Mutex<Vec<RecordedCall>>, invocation: &GenerateContentCall<'_>) {
    recorded_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(recorded(invocation));
}

#[async_trait]
impl GenerateContentDispatch for FakeGeneration {
    async fn generate(
        &self,
        call: GenerateContentCall<'_>,
    ) -> Result<GenerationAnswer, Box<ProxyException>> {
        record(&self.generate_calls, &call);
        take(&self.generate)
    }

    async fn stream(
        &self,
        call: GenerateContentCall<'_>,
    ) -> Result<GenerationAnswer, Box<ProxyException>> {
        record(&self.stream_calls, &call);
        take(&self.stream)
    }
}

pub(super) struct FakeTranslator {
    messages: Mutex<Option<Result<Vec<ChatMessage>, Box<ProxyException>>>>,
    calls: Mutex<Vec<(String, Json)>>,
}

impl FakeTranslator {
    fn new(messages: Option<Result<Vec<ChatMessage>, Box<ProxyException>>>) -> Arc<Self> {
        Arc::new(Self {
            messages: Mutex::new(messages),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn silent() -> Arc<Self> {
        Self::new(None)
    }

    pub(super) fn answering(messages: Vec<ChatMessage>) -> Arc<Self> {
        Self::new(Some(Ok(messages)))
    }

    pub(super) fn failing(exception: ProxyException) -> Arc<Self> {
        Self::new(Some(Err(Box::new(exception))))
    }

    pub(super) fn calls(&self) -> Vec<(String, Json)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TranslateContents for FakeTranslator {
    fn messages(
        &self,
        model: &str,
        contents: &Json,
    ) -> Result<Vec<ChatMessage>, Box<ProxyException>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), contents.clone()));
        self.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Err(Box::new(unscripted())))
    }
}

pub(super) struct FakeCounter {
    outcome: Mutex<Option<Result<Option<TokenCountResponse>, Box<ProxyException>>>>,
    calls: Mutex<Vec<(TokenCountRequest, bool)>>,
}

impl FakeCounter {
    fn new(outcome: Option<Result<Option<TokenCountResponse>, Box<ProxyException>>>) -> Arc<Self> {
        Arc::new(Self {
            outcome: Mutex::new(outcome),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn silent() -> Arc<Self> {
        Self::new(None)
    }

    pub(super) fn returning(response: Option<TokenCountResponse>) -> Arc<Self> {
        Self::new(Some(Ok(response)))
    }

    pub(super) fn failing(exception: ProxyException) -> Arc<Self> {
        Self::new(Some(Err(Box::new(exception))))
    }

    pub(super) fn calls(&self) -> Vec<(TokenCountRequest, bool)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl TokenCounter for FakeCounter {
    async fn count_tokens(
        &self,
        request: &TokenCountRequest,
        call_endpoint: bool,
    ) -> Result<Option<TokenCountResponse>, Box<ProxyException>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request.clone(), call_endpoint));
        self.outcome
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Err(Box::new(unscripted())))
    }
}

pub(super) fn detail(modality: Modality, token_count: i64) -> PromptTokenDetail {
    PromptTokenDetail {
        modality,
        token_count,
    }
}

pub(super) fn gen_router(generation: Arc<FakeGeneration>) -> Router {
    routes(GenaiRoutesState {
        generation,
        translator: FakeTranslator::silent(),
        token_counter: FakeCounter::silent(),
    })
}

pub(super) fn count_router(translator: Arc<FakeTranslator>, counter: Arc<FakeCounter>) -> Router {
    routes(GenaiRoutesState {
        generation: FakeGeneration::silent(),
        translator,
        token_counter: counter,
    })
}

pub(super) fn post(path: &str, body: &str) -> Request<Body> {
    authorised("POST", path, Body::from(body.to_owned()))
}

pub(super) fn post_bytes(path: &str, body: Vec<u8>) -> Request<Body> {
    authorised("POST", path, Body::from(body))
}

pub(super) fn get(path: &str) -> Request<Body> {
    authorised("GET", path, Body::empty())
}

// the same POST without the AuthContext B003's key auth middleware inserts
pub(super) fn unauthenticated(path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn authorised(method: &str, path: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .extension(auth_context())
        .body(body)
        .unwrap()
}

pub(super) async fn call(router: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body)
}

pub(super) fn json_body(body: &Bytes) -> Json {
    serde_json::from_slice(body).unwrap()
}

pub(super) fn parse(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn error_body(message: &str, error_type: &str, param: Option<&str>, code: &str) -> Json {
    let mut fields = Object::new();
    fields.insert("message".to_owned(), text(message));
    fields.insert("type".to_owned(), text(error_type));
    fields.insert("param".to_owned(), param.map_or(Json::Null, text));
    fields.insert("code".to_owned(), text(code));
    Json::Object(fields)
}
