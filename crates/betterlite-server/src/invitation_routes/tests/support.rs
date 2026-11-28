#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_pyhost::json_values::JsonValue;
use betterlite_router::request_scheduler::service::RequestScheduler;
use betterlite_router::router_core::headers;
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use futures_util::stream;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::invitation_routes::routes;
use crate::invitation_routes::state::{
    InvitationDatabase, InvitationRouteState, InvitationScope, QueueAnswer, QueueCompletion,
    QueueFailureHook, QueueRouteState, QueueRouter,
};
use crate::invitation_routes::store::{InvitationStore, InvitationStoreError};
use crate::pre_call::rewrites::CliOverrides;
use crate::routing::request_scheduler::build;
use crate::sse_data_generator::types::{StreamFailure, StreamSource};
use crate::stream_chunks::StreamItem;
use crate::user_invitation::{InvitationError, InvitationRecord};

pub(super) fn parse(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the body is JSON")
}

pub(super) fn moment(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).expect("a moment")
}

pub(super) fn record(id: &str, user_id: &str, created_by: &str) -> InvitationRecord {
    InvitationRecord {
        id: id.to_owned(),
        user_id: user_id.to_owned(),
        is_accepted: false,
        accepted_at: None,
        expires_at: moment(1_800_000_000),
        created_at: moment(1_780_000_000),
        created_by: created_by.to_owned(),
        updated_at: moment(1_780_000_000),
        updated_by: created_by.to_owned(),
    }
}

pub(super) fn caller(principal: &str) -> AuthContext {
    let decoded: Principal = serde_json::from_str(principal).expect("a principal");
    AuthContext { principal: decoded }
}

pub(super) fn master() -> AuthContext {
    master_caller()
}

pub(super) fn platform_admin() -> AuthContext {
    caller(
        r#"{"principal_type":"human","subject":"admin","auth_method":"api_key","roles":["platform_admin"],"user":{"id":"admin-user"}}"#,
    )
}

pub(super) fn platform_viewer() -> AuthContext {
    caller(
        r#"{"principal_type":"human","subject":"viewer","auth_method":"api_key","roles":["platform_viewer"],"user":{"id":"viewer"}}"#,
    )
}

pub(super) fn org_admin() -> AuthContext {
    caller(
        r#"{"principal_type":"human","subject":"org-admin","auth_method":"api_key","roles":["org_admin"],"user":{"id":"org-admin"}}"#,
    )
}

pub(super) fn internal_user() -> AuthContext {
    caller(
        r#"{"principal_type":"human","subject":"member","auth_method":"api_key","user":{"id":"member"}}"#,
    )
}

pub(super) fn userless_admin() -> AuthContext {
    caller(
        r#"{"principal_type":"service_account","subject":"admin","auth_method":"api_key","roles":["platform_admin"]}"#,
    )
}

pub(super) fn key_caller(fields: &str) -> AuthContext {
    with_key(
        r#"{"principal_type":"service_account","subject":"key-owner","auth_method":"api_key"}"#,
        fields,
    )
}

pub(super) fn with_key(principal: &str, fields: &str) -> AuthContext {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).expect("key fields");
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("a key");
    let mut context = caller(principal);
    context.principal.source_key = SourceKey(Some(key));
    context
}

pub(super) async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    body: &str,
    who: Option<AuthContext>,
) -> (StatusCode, HeaderMap, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("host", "proxy.test");
    if let Some(context) = who {
        builder = builder.extension(context);
    }
    let request = builder
        .body(Body::from(body.to_owned()))
        .expect("a request");
    send_with(router, request).await
}

pub(super) async fn send_with(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, HeaderMap, String) {
    let response = router.clone().oneshot(request).await.expect("a response");
    let status = response.status();
    let response_headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    let text = String::from_utf8(bytes.to_vec()).expect("utf8");
    (status, response_headers, text)
}

#[derive(Default)]
pub(super) struct StoreRows {
    pub(super) records: BTreeMap<String, InvitationRecord>,
    pub(super) created: Vec<(String, Option<String>)>,
    pub(super) updated: Vec<(String, bool, String)>,
    pub(super) deleted: Vec<String>,
    pub(super) create_error: Option<InvitationError>,
    pub(super) read_error: Option<DbError>,
    pub(super) update_error: Option<DbError>,
    pub(super) delete_error: Option<DbError>,
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub(super) rows: Mutex<StoreRows>,
}

impl FakeStore {
    pub(super) fn with_records(records: Vec<InvitationRecord>) -> Self {
        let store = Self::default();
        {
            let mut rows = store.rows.lock().expect("the store lock");
            for record in records {
                rows.records.insert(record.id.clone(), record);
            }
        }
        store
    }
}

type Created = Result<InvitationRecord, InvitationStoreError>;
type Found = Result<Option<InvitationRecord>, InvitationStoreError>;

#[async_trait]
impl InvitationStore for FakeStore {
    async fn create(&self, user_id: &str, actor_user_id: Option<&str>) -> Created {
        let mut rows = self.rows.lock().expect("the store lock");
        if let Some(error) = rows.create_error.take() {
            return Err(InvitationStoreError::Create(error));
        }
        rows.created
            .push((user_id.to_owned(), actor_user_id.map(str::to_owned)));
        drop(rows);
        Ok(record(
            "fresh-id",
            user_id,
            actor_user_id.unwrap_or("default_user_id"),
        ))
    }

    async fn info(&self, invitation_id: &str) -> Found {
        let mut rows = self.rows.lock().expect("the store lock");
        if let Some(error) = rows.read_error.take() {
            return Err(InvitationStoreError::Read(error));
        }
        Ok(rows.records.get(invitation_id).cloned())
    }

    async fn update(&self, invitation_id: &str, is_accepted: bool, updated_by: &str) -> Found {
        let found = {
            let mut rows = self.rows.lock().expect("the store lock");
            if let Some(error) = rows.update_error.take() {
                return Err(InvitationStoreError::Update(error));
            }
            rows.updated
                .push((invitation_id.to_owned(), is_accepted, updated_by.to_owned()));
            rows.records.get_mut(invitation_id).map(|row| {
                row.is_accepted = is_accepted;
                row.updated_by = updated_by.to_owned();
                row.clone()
            })
        };
        Ok(found)
    }

    async fn delete(&self, invitation_id: &str) -> Found {
        let mut rows = self.rows.lock().expect("the store lock");
        if let Some(error) = rows.delete_error.take() {
            return Err(InvitationStoreError::Delete(error));
        }
        rows.deleted.push(invitation_id.to_owned());
        Ok(rows.records.remove(invitation_id))
    }
}

#[derive(Clone, Copy)]
pub(super) enum ScopeRights {
    Open,
    Privileged,
    Refused,
}

pub(super) struct FakeScope {
    pub(super) privileged: bool,
    pub(super) can_invite: bool,
    pub(super) calls: Mutex<Vec<String>>,
}

impl FakeScope {
    pub(super) fn new(rights: ScopeRights) -> Self {
        let (privileged, can_invite) = match rights {
            ScopeRights::Open => (true, true),
            ScopeRights::Privileged => (true, false),
            ScopeRights::Refused => (false, false),
        };
        Self {
            privileged,
            can_invite,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the scope lock").clone()
    }
}

#[async_trait]
impl InvitationScope for FakeScope {
    async fn has_admin_privileges(&self, _caller: &Principal) -> bool {
        self.calls
            .lock()
            .expect("the scope lock")
            .push("has_admin_privileges".to_owned());
        self.privileged
    }

    async fn can_invite_user(&self, _caller: &Principal, target_user_id: &str) -> bool {
        self.calls
            .lock()
            .expect("the scope lock")
            .push(format!("can_invite_user:{target_user_id}"));
        self.can_invite
    }
}

pub(super) fn invitation_router(
    store: &Arc<FakeStore>,
    scope: &Arc<FakeScope>,
) -> InvitationRouteState {
    let store_handle: Arc<FakeStore> = Arc::clone(store);
    let invitations: Arc<dyn InvitationStore> = store_handle;
    let scope_handle: Arc<FakeScope> = Arc::clone(scope);
    let invitation_scope: Arc<dyn InvitationScope> = scope_handle;
    InvitationRouteState {
        invitations: Some(InvitationDatabase {
            invitations,
            scope: invitation_scope,
        }),
        queue: QueueRouteState::default(),
    }
}

pub(super) fn router_for(state: InvitationRouteState) -> Router {
    routes(state)
}

pub(super) enum Reply {
    Object(JsonObject),
    Stream,
    Failure(String),
}

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

#[derive(Default)]
pub(super) struct CompletionState {
    pub(super) calls: Vec<(JsonMap, bool)>,
    pub(super) reply: Option<Reply>,
    pub(super) items: Option<Vec<Result<StreamItem, StreamFailure>>>,
}

pub(super) struct FakeCompletion {
    pub(super) healthy: usize,
    pub(super) state: Mutex<CompletionState>,
}

impl FakeCompletion {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            healthy: 1,
            state: Mutex::new(CompletionState {
                calls: Vec::new(),
                reply: Some(reply),
                items: None,
            }),
        }
    }

    pub(super) fn streaming(items: Vec<Result<StreamItem, StreamFailure>>) -> Self {
        let completion = Self::new(Reply::Object(object("{}")));
        completion.state.lock().expect("the completion lock").items = Some(items);
        completion
    }

    pub(super) fn calls(&self) -> Vec<(JsonMap, bool)> {
        self.state
            .lock()
            .expect("the completion lock")
            .calls
            .clone()
    }

    pub(super) fn seen(&self) -> JsonMap {
        self.calls()
            .first()
            .map(|(data, _stream)| data.clone())
            .expect("the completion ran")
    }
}

pub(super) fn failure(message: &str) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

#[async_trait]
impl QueueCompletion for FakeCompletion {
    fn healthy_deployments(&self, _model: &str) -> usize {
        self.healthy
    }

    async fn complete(
        &self,
        _caller: &Principal,
        data: &JsonMap,
        stream: bool,
    ) -> Result<QueueAnswer, Box<ErrorClass>> {
        let (reply, items) = {
            let mut state = self.state.lock().expect("the completion lock");
            state.calls.push((data.clone(), stream));
            let reply = state.reply.take().unwrap_or(Reply::Object(object("{}")));
            (reply, state.items.take())
        };
        match reply {
            Reply::Failure(message) => Err(Box::new(failure(&message))),
            Reply::Stream => Ok(QueueAnswer {
                response: headers::Response::Stream,
                stream: None,
            }),
            Reply::Object(body) => {
                let source = items.map(|queued| StreamSource {
                    items: stream::iter(queued).boxed(),
                    hidden: JsonObject::new(),
                    snapshot: None,
                });
                Ok(QueueAnswer {
                    response: headers::Response::Object(body),
                    stream: source,
                })
            }
        }
    }
}

pub(super) struct FakeHook {
    pub(super) data: Mutex<Vec<JsonMap>>,
    pub(super) seen: Mutex<Vec<(String, String)>>,
    pub(super) replacement: Option<ProxyException>,
}

impl FakeHook {
    pub(super) fn new(replacement: Option<ProxyException>) -> Self {
        Self {
            data: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            replacement,
        }
    }

    pub(super) fn call_count(&self) -> usize {
        self.data.lock().expect("the hook lock").len()
    }

    pub(super) fn seen(&self) -> Vec<(String, String)> {
        self.seen.lock().expect("the hook lock").clone()
    }
}

#[async_trait]
impl QueueFailureHook for FakeHook {
    async fn post_call_failure(
        &self,
        _caller: &Principal,
        exception: &ProxyException,
        request_data: &JsonMap,
    ) -> Option<ProxyException> {
        self.data
            .lock()
            .expect("the hook lock")
            .push(request_data.clone());
        self.seen
            .lock()
            .expect("the hook lock")
            .push((exception.message.clone(), exception.code.clone()));
        self.replacement.clone()
    }
}

pub(super) fn scheduler() -> Arc<RequestScheduler> {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("model_list: []\n")
    .expect("the scheduler config loads");
    build(&config).expect("a scheduler")
}

pub(super) fn queue_state(
    scheduler: &Arc<RequestScheduler>,
    completion: &Arc<FakeCompletion>,
    failure: Option<Arc<dyn QueueFailureHook>>,
) -> QueueRouteState {
    let completion_handle: Arc<FakeCompletion> = Arc::clone(completion);
    let queued: Arc<dyn QueueCompletion> = completion_handle;
    QueueRouteState {
        router: Some(QueueRouter {
            scheduler: Arc::clone(scheduler),
            completion: queued,
        }),
        keepalive: None,
        failure,
        completion_model: None,
        cli_model: None,
        overrides: CliOverrides::default(),
        timeouts: QueueRouteState::default().timeouts,
    }
}

pub(super) fn queue_router(state: QueueRouteState) -> Router {
    routes(InvitationRouteState {
        invitations: None,
        queue: state,
    })
}
