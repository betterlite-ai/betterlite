use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::future::{Future, Ready, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{Body, BodyDataStream, to_bytes};
use axum::http::{Method, Request, Response};
use betterlite_analytics::daily_metrics::{DailySpendMetadata, DailySpendPage};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use chrono::NaiveDate;
use futures_util::StreamExt as _;
use futures_util::stream::{self, Stream};
use tokio::sync::Notify;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::keepalive_interval::PingSetting;
use crate::management_guards::error::ManagementGuardError;
use crate::usage_ai_chat::messages::ChatMessage;
use crate::usage_ai_chat::model::{ModelReply, ModelRequest, ModelToolCall, UsageChatModel};
use crate::usage_ai_chat::parts::UsageChatParts;
use crate::usage_ai_chat::source::{Ids, PageResult, UsageDataSource};
use crate::usage_ai_chat::tools::ToolDefinition;
use crate::usage_ai_chat_route::caller::NonAdminCallerId;
use crate::usage_ai_chat_route::routes;
use crate::usage_ai_chat_route::state::{UsageAiChatState, UsageChatFactory};

pub(crate) const THINKING_FRAME: &str =
    "data: {\"type\": \"status\", \"message\": \"Thinking...\"}\n\n";
pub(crate) const ANSWER_FRAME: &str = "data: {\"type\": \"chunk\", \"content\": \"Hi\"}\n\n";
pub(crate) const DONE_FRAME: &str = "data: {\"type\": \"done\"}\n\n";
pub(crate) const PING_FRAME: &str = ": ping\n\n";
pub(crate) const USAGE_ARGS: &str =
    "{\"start_date\": \"2026-01-01\", \"end_date\": \"2026-01-31\"}";

pub(crate) fn answer_body() -> String {
    format!("{THINKING_FRAME}{ANSWER_FRAME}{DONE_FRAME}")
}

pub(crate) fn answer_reply(content: &str) -> ModelReply {
    ModelReply {
        content: Some(content.to_owned()),
        tool_calls: Vec::new(),
    }
}

pub(crate) fn usage_tool_reply() -> ModelReply {
    ModelReply {
        content: None,
        tool_calls: vec![ModelToolCall {
            id: "call-1".to_owned(),
            name: "get_usage_data".to_owned(),
            arguments: USAGE_ARGS.to_owned(),
        }],
    }
}

pub(crate) fn empty_page() -> DailySpendPage {
    DailySpendPage {
        results: Vec::new(),
        metadata: DailySpendMetadata::default(),
    }
}

pub(crate) fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 16).expect("the fixture date")
}

#[derive(Clone)]
pub(crate) struct FakeModel {
    state: Arc<ModelState>,
}

struct ModelState {
    replies: Mutex<VecDeque<ModelReply>>,
    gate: Mutex<Option<Arc<Notify>>>,
    calls: Mutex<Vec<ModelCall>>,
}

#[derive(Clone)]
pub(crate) struct ModelCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) tools: usize,
}

impl FakeModel {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(ModelState {
                replies: Mutex::new(VecDeque::new()),
                gate: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
            }),
        }
    }

    pub(crate) fn replying(self, reply: ModelReply) -> Self {
        self.state
            .replies
            .lock()
            .expect("the reply queue")
            .push_back(reply);
        self
    }

    pub(crate) fn waiting(self, gate: Arc<Notify>) -> Self {
        *self.state.gate.lock().expect("the model gate") = Some(gate);
        self.replying(answer_reply("Hi"))
    }

    pub(crate) fn calls(&self) -> Vec<ModelCall> {
        self.state.calls.lock().expect("the model call log").clone()
    }

    fn record(&self, request: &ModelRequest<'_>) {
        let call = ModelCall {
            model: request.model.to_owned(),
            messages: request.messages.to_vec(),
            tools: request.tools.map_or(0, <[ToolDefinition]>::len),
        };
        self.state
            .calls
            .lock()
            .expect("the model call log")
            .push(call);
    }
}

impl UsageChatModel for FakeModel {
    type Error = Infallible;

    fn complete(
        &self,
        request: &ModelRequest<'_>,
    ) -> impl Future<Output = Result<ModelReply, Infallible>> + Send {
        self.record(request);
        let reply = self
            .state
            .replies
            .lock()
            .expect("the reply queue")
            .pop_front()
            .expect("a scripted reply");
        let gate = self.state.gate.lock().expect("the model gate").clone();
        async move {
            if let Some(notify) = gate {
                notify.notified().await;
            }
            Ok(reply)
        }
    }

    fn stream(
        &self,
        _request: &ModelRequest<'_>,
    ) -> impl Future<
        Output = Result<impl Stream<Item = Result<String, Infallible>> + Send, Infallible>,
    > + Send {
        let items: Vec<Result<String, Infallible>> = Vec::new();
        ready(Ok(stream::iter(items)))
    }
}

#[derive(Clone)]
pub(crate) struct FakeSource {
    state: Arc<SourceState>,
}

struct SourceState {
    user_ids: Mutex<Vec<Option<String>>>,
}

impl FakeSource {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(SourceState {
                user_ids: Mutex::new(Vec::new()),
            }),
        }
    }

    pub(crate) fn user_ids(&self) -> Vec<Option<String>> {
        self.state
            .user_ids
            .lock()
            .expect("the source call log")
            .clone()
    }

    fn empty() -> Ready<PageResult> {
        ready(PageResult::Ok(empty_page()))
    }
}

impl UsageDataSource for FakeSource {
    async fn global(
        &self,
        _start_date: &str,
        _end_date: &str,
        user_id: Option<&str>,
    ) -> PageResult {
        self.state
            .user_ids
            .lock()
            .expect("the source call log")
            .push(user_id.map(str::to_owned));
        Self::empty().await
    }

    async fn teams(&self, _start_date: &str, _end_date: &str, _ids: Ids<'_>) -> PageResult {
        Self::empty().await
    }

    async fn tags(&self, _start_date: &str, _end_date: &str, _tags: Ids<'_>) -> PageResult {
        Self::empty().await
    }
}

#[derive(Clone)]
pub(crate) struct FakeFactory {
    model: FakeModel,
    data: FakeSource,
}

impl FakeFactory {
    pub(crate) fn new(model: FakeModel, data: FakeSource) -> Self {
        Self { model, data }
    }
}

impl UsageChatFactory for FakeFactory {
    type Model = FakeModel;
    type Data = FakeSource;
    type Clock = fn() -> NaiveDate;

    fn parts(&self) -> UsageChatParts<FakeModel, FakeSource, fn() -> NaiveDate> {
        UsageChatParts {
            model: self.model.clone(),
            data: self.data.clone(),
            clock: today,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FakeCallerId {
    calls: Arc<AtomicUsize>,
}

impl FakeCallerId {
    pub(crate) fn from_subject() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl NonAdminCallerId for FakeCallerId {
    fn user_id_for_non_admin<'principal>(
        &self,
        principal: &'principal Principal,
    ) -> Result<&'principal str, ManagementGuardError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(principal.subject.as_str())
    }
}

pub(crate) fn principal(subject: &str, user: Option<&str>, roles: &[Role]) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: subject.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: user.map(|id| UserIdentity {
            id: id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: roles.to_vec(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

pub(crate) fn admin_principal(user: &str) -> Principal {
    principal(user, Some(user), &[Role::PlatformAdmin])
}

pub(crate) fn master_principal() -> Principal {
    master_caller().principal
}

pub(crate) fn key_admin_principal(subject: &str) -> Principal {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(
        "user_role".to_owned(),
        JsonData::String("proxy_admin_viewer".to_owned()),
    );
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture");
    Principal {
        source_key: SourceKey(Some(key)),
        ..principal(subject, None, &[])
    }
}

pub(crate) fn state<Caller>(
    factory: FakeFactory,
    ping_interval: Option<PingSetting>,
    caller: Caller,
) -> UsageAiChatState<FakeFactory>
where
    Caller: NonAdminCallerId + 'static,
{
    UsageAiChatState {
        factory: Arc::new(factory),
        ping_interval,
        caller_id: Arc::new(caller),
    }
}

pub(crate) fn post(body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/usage/ai/chat")
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("the request builds")
}

pub(crate) fn with_principal(request: Request<Body>, principal: Principal) -> Request<Body> {
    let (mut parts, body) = request.into_parts();
    parts.extensions.insert(AuthContext { principal });
    Request::from_parts(parts, body)
}

pub(crate) async fn send(
    state: UsageAiChatState<FakeFactory>,
    request: Request<Body>,
) -> Response<Body> {
    routes(state)
        .oneshot(request)
        .await
        .expect("the route answers")
}

pub(crate) async fn body_text(response: Response<Body>) -> String {
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the response body");
    String::from_utf8(bytes.to_vec()).expect("a utf-8 body")
}

pub(crate) type Frames = Pin<Box<BodyDataStream>>;

pub(crate) fn frames(response: Response<Body>) -> Frames {
    Box::pin(response.into_body().into_data_stream())
}

pub(crate) async fn next_frame(frames: &mut Frames) -> Option<String> {
    let chunk = frames.next().await?;
    let bytes = chunk.expect("a frame chunk");
    Some(String::from_utf8(bytes.to_vec()).expect("a utf-8 frame"))
}

pub(crate) fn parsed(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("valid json")
}
