#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_config::sections::request_body::Settings;
use betterlite_core::agents::json::AgentJson as JsonValue;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tower::ServiceExt as _;

use crate::api::agents::collection_routes::processing::{AgentCall, AgentRequestProcessing};
use crate::api::agents::collection_routes::{AgentCollectionState, routes};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::pre_call::failure::{PreCallError, PreCallFailure};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::route_type::RouteType;
use crate::pre_call_step::{PreCall, PreCallInput, PreCallOutput};

pub(crate) const VERSION: &str = "1.105.1";
pub(crate) const TEAM: &str = "team-1";

#[derive(Clone, Default)]
pub(crate) struct TestKey {
    team: Option<String>,
}

impl PreCallKey for TestKey {
    fn team_id(&self) -> Option<&str> {
        self.team.as_deref()
    }

    fn key_aliases(&self) -> Option<&JsonMap> {
        None
    }

    fn key_router_settings(&self) -> Option<&JsonValue> {
        None
    }
}

pub(crate) fn key() -> TestKey {
    TestKey {
        team: Some(TEAM.to_owned()),
    }
}

pub(crate) fn principal(subject: &str, roles: &[Role]) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: subject.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
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

pub(crate) fn context(subject: &str, roles: &[Role]) -> AuthContext {
    AuthContext {
        principal: principal(subject, roles),
    }
}

// a caller no guard lets through and no test mistakes for an admin
pub(crate) fn user() -> AuthContext {
    context("user-1", &[])
}

pub(crate) fn admin() -> AuthContext {
    context("admin-1", &[Role::PlatformAdmin])
}

pub(crate) fn master_key() -> AuthContext {
    master_caller()
}

pub(crate) fn json_map(text: &str) -> JsonMap {
    serde_json::from_str(text).unwrap()
}

// a whole answer is JSON text, so the object's value is the parsed map
pub(crate) fn json_object(text: &str) -> JsonValue {
    JsonValue::Object(json_map(text))
}

pub(crate) fn json_text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

pub(crate) fn query(pairs: &[(&str, &str)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(pairs.iter().copied());
    serializer.finish()
}

pub(crate) fn start_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: "fixture-model".to_owned(),
        call_type: "acreate_agent".to_owned(),
        start_time: start_time(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

pub(crate) fn logging_state() -> LoggingState {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        logging_init(),
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state")
}

#[derive(Clone)]
enum PreCallReply {
    PassThrough,
    Replace(JsonMap),
    Fail(PreCallFailure),
}

#[derive(Clone)]
pub(crate) struct RecordedPreCall {
    pub(crate) data: JsonMap,
    pub(crate) route_type: RouteType,
    pub(crate) version: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) skip_guardrails: bool,
    pub(crate) team: Option<String>,
}

pub(crate) struct FakePreCall {
    reply: PreCallReply,
    calls: Mutex<Vec<RecordedPreCall>>,
}

impl FakePreCall {
    fn new(reply: PreCallReply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn passing() -> Arc<Self> {
        Self::new(PreCallReply::PassThrough)
    }

    pub(crate) fn replacing(data: JsonMap) -> Arc<Self> {
        Self::new(PreCallReply::Replace(data))
    }

    pub(crate) fn failing(failure: PreCallFailure) -> Arc<Self> {
        Self::new(PreCallReply::Fail(failure))
    }

    pub(crate) fn calls(&self) -> Vec<RecordedPreCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PreCall<TestKey> for FakePreCall {
    async fn pre_call(
        &self,
        input: PreCallInput<'_, TestKey>,
    ) -> Result<PreCallOutput, PreCallError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedPreCall {
                data: input.data.clone(),
                route_type: input.route_type,
                version: input.version.map(str::to_owned),
                model: input.model.map(str::to_owned),
                skip_guardrails: input.skip_guardrails,
                team: input.key.team_id().map(str::to_owned),
            });
        match &self.reply {
            PreCallReply::PassThrough => Ok(PreCallOutput {
                data: input.data,
                logging: logging_state(),
            }),
            PreCallReply::Replace(replacement) => Ok(PreCallOutput {
                data: replacement.clone(),
                logging: logging_state(),
            }),
            PreCallReply::Fail(failure) => Err(PreCallError {
                failure: failure.clone(),
                data: input.data,
            }),
        }
    }
}

#[derive(Clone)]
enum ProcessReply {
    Answer { status: u16, body: JsonValue },
    Fail(PreCallFailure),
}

#[derive(Clone)]
pub(crate) struct RecordedCall {
    pub(crate) data: JsonMap,
    pub(crate) route_type: RouteType,
    pub(crate) version: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) team: Option<String>,
}

#[derive(Clone)]
pub(crate) struct RecordedFailure {
    pub(crate) failure: PreCallFailure,
    pub(crate) data: JsonMap,
}

pub(crate) struct FakeProcessing {
    reply: ProcessReply,
    calls: Mutex<Vec<RecordedCall>>,
    failures: Mutex<Vec<RecordedFailure>>,
}

impl FakeProcessing {
    fn new(reply: ProcessReply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            calls: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn answering(body: JsonValue) -> Arc<Self> {
        Self::new(ProcessReply::Answer { status: 200, body })
    }

    pub(crate) fn failing(failure: PreCallFailure) -> Arc<Self> {
        Self::new(ProcessReply::Fail(failure))
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn failures(&self) -> Vec<RecordedFailure> {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn failure_message(failure: &PreCallFailure) -> String {
    match failure {
        PreCallFailure::Proxy(proxy) | PreCallFailure::RateLimited(proxy) => proxy.message.clone(),
        PreCallFailure::Http { detail, .. } => detail.to_string(),
    }
}

#[async_trait]
impl AgentRequestProcessing<TestKey> for FakeProcessing {
    async fn process(&self, call: AgentCall<'_, TestKey>) -> Result<Response, PreCallFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                data: call.data.clone(),
                route_type: call.route_type,
                version: call.version.to_owned(),
                method: call.request.method.as_str().to_owned(),
                path: call.request.uri.path().to_owned(),
                team: call.key.team_id().map(str::to_owned),
            });
        match &self.reply {
            ProcessReply::Answer { status, body } => {
                let answered = StatusCode::from_u16(*status).unwrap_or(StatusCode::OK);
                Ok((answered, Json(body.clone())).into_response())
            }
            ProcessReply::Fail(failure) => Err(failure.clone()),
        }
    }

    async fn handle_exception(
        &self,
        failure: PreCallFailure,
        data: &JsonMap,
        key: &TestKey,
    ) -> Response {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedFailure {
                failure: failure.clone(),
                data: data.clone(),
            });
        let team = key
            .team_id()
            .map_or(JsonValue::Null, |team| JsonValue::String(team.to_owned()));
        let mut body = JsonMap::new();
        body.insert(String::from("handled"), JsonValue::Bool(true));
        body.insert(
            String::from("message"),
            JsonValue::String(failure_message(&failure)),
        );
        body.insert(String::from("team"), team);
        (StatusCode::BAD_GATEWAY, Json(body)).into_response()
    }
}

pub(crate) fn router(pre_call: Arc<FakePreCall>, processing: Arc<FakeProcessing>) -> Router {
    routes(AgentCollectionState {
        pre_call,
        processing,
        request_body: Settings::default(),
        limits: ApiLimits::default(),
        version: VERSION.to_owned(),
    })
}

pub(crate) fn request(
    method: Method,
    uri: &str,
    body: Option<&str>,
    context: AuthContext,
    key: TestKey,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    builder
        .extension(context)
        .extension(key)
        .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
        .unwrap()
}

pub(crate) async fn call(router: Router, request: Request<Body>) -> (StatusCode, JsonValue) {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(JsonValue::Null);
    (status, body)
}
