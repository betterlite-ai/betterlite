use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, OrganizationIdentity, Principal, ProjectIdentity, SourceKey,
    TeamIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role, TeamRole};
use betterlite_logging::otlp_normalize::{DecodedSpan, NormalizedSpan, SpanRow};
use betterlite_logging::trace_types::{
    SpanDetail, SpanErrorPage, SpanType, Trace, TracePage, TraceScope,
};
use betterlite_tracing::storage::trace_store::SpanErrorTarget;
use indexmap::IndexMap;
use secrecy::SecretString;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::tracing_routes::TraceRoutesState;
use crate::tracing_routes::access::{Tenant, TraceClock};
use crate::tracing_routes::query::{QueryRunError, QueryScope, TraceQueryStore};
use crate::tracing_routes::receiver::{IngestError, TraceReadError, TraceRuntime};
use crate::tracing_routes::router;

pub(super) struct FixedClock;

impl TraceClock for FixedClock {
    fn now_ms(&self) -> i64 {
        1_700_000_000_000
    }
}

#[derive(Clone, Debug)]
pub(super) enum IngestAnswer {
    Ok(usize),
    TooLarge(String),
    Invalid(String),
    Storage,
}

#[derive(Clone, Debug)]
pub(super) enum ReadAnswer<T> {
    Ok(T),
    InvalidCursor(String),
    Storage,
}

#[derive(Clone, Debug)]
pub(super) enum QueryAnswer {
    Ok(String),
    Invalid(String),
    Unavailable(String),
}

#[derive(Clone, Debug)]
pub(super) struct SeenIngest {
    pub(super) content_type: Option<String>,
    pub(super) content_encoding: Option<String>,
    pub(super) tenant: Tenant,
    pub(super) body_len: usize,
}

#[derive(Clone, Debug)]
pub(super) struct SeenList {
    pub(super) scope: TraceScope,
    pub(super) start_ms: i64,
    pub(super) end_ms: i64,
    pub(super) cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct SeenSpan {
    pub(super) trace_id: String,
    pub(super) span_id: String,
    pub(super) scope: TraceScope,
    pub(super) trace_ref: String,
}

#[derive(Clone, Debug)]
pub(super) struct SeenError {
    pub(super) trace_id: String,
    pub(super) span_id: String,
    pub(super) scope: TraceScope,
    pub(super) trace_ref: String,
    pub(super) cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct SeenQuery {
    pub(super) sql: Option<String>,
    pub(super) scope: QueryScope,
    pub(super) secret: String,
}

pub(super) struct FakeQueryStore {
    pub(super) sql: QueryAnswer,
    pub(super) help: QueryAnswer,
    pub(super) seen: Mutex<Vec<SeenQuery>>,
}

impl FakeQueryStore {
    pub(super) fn new(sql: QueryAnswer, help: QueryAnswer) -> Self {
        Self {
            sql,
            help,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn answer(
        &self,
        sql: Option<String>,
        scope: &QueryScope,
        secret: &SecretString,
        reply: &QueryAnswer,
    ) -> Result<String, QueryRunError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenQuery {
                sql,
                scope: scope.clone(),
                secret: secret_text(secret),
            });
        match reply {
            QueryAnswer::Ok(result) => Ok(result.clone()),
            QueryAnswer::Invalid(message) => Err(QueryRunError::Invalid(message.clone())),
            QueryAnswer::Unavailable(message) => Err(QueryRunError::Unavailable(message.clone())),
        }
    }
}

#[async_trait]
impl TraceQueryStore for FakeQueryStore {
    async fn query_sql(
        &self,
        sql: &str,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        let owned = Some(sql.to_owned());
        self.answer(owned, scope, secret, &self.sql)
    }

    async fn query_help(
        &self,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        self.answer(None, scope, secret, &self.help)
    }
}

pub(super) fn secret_text(secret: &SecretString) -> String {
    use secrecy::ExposeSecret as _;
    secret.expose_secret().to_owned()
}

pub(super) struct FakeRuntime {
    pub(super) ingest: IngestAnswer,
    pub(super) list: ReadAnswer<TracePage>,
    pub(super) trace: ReadAnswer<Option<Trace>>,
    pub(super) span: ReadAnswer<Option<SpanDetail>>,
    pub(super) span_error: ReadAnswer<Option<SpanErrorPage>>,
    pub(super) queries: Arc<FakeQueryStore>,
    pub(super) seen_ingest: Mutex<Option<SeenIngest>>,
    pub(super) seen_list: Mutex<Option<SeenList>>,
    pub(super) seen_trace: Mutex<Option<(String, String)>>,
    pub(super) seen_span: Mutex<Option<SeenSpan>>,
    pub(super) seen_error: Mutex<Option<SeenError>>,
}

impl FakeRuntime {
    fn record_trace(&self, trace: &str, reference: &str) {
        let seen = (trace.to_owned(), reference.to_owned());
        *self.seen_trace.lock().expect("lock") = Some(seen);
    }

    pub(super) fn new() -> Self {
        let ingest = IngestAnswer::Ok(0);
        let list = ReadAnswer::Ok(TracePage {
            data: Vec::new(),
            next_cursor: None,
        });
        let trace = ReadAnswer::Ok(None);
        let span = ReadAnswer::Ok(None);
        let span_error = ReadAnswer::Ok(None);
        let queries = Arc::new(FakeQueryStore::new(
            QueryAnswer::Ok("{}".to_owned()),
            QueryAnswer::Ok("{}".to_owned()),
        ));
        let seen_ingest = Mutex::new(None);
        let seen_list = Mutex::new(None);
        let seen_trace = Mutex::new(None);
        let seen_span = Mutex::new(None);
        let seen_error = Mutex::new(None);
        Self {
            ingest,
            list,
            trace,
            span,
            span_error,
            queries,
            seen_ingest,
            seen_list,
            seen_trace,
            seen_span,
            seen_error,
        }
    }
}

#[async_trait]
impl TraceRuntime for FakeRuntime {
    async fn ingest(
        &self,
        body: Body,
        content_type: Option<&str>,
        content_encoding: Option<&str>,
        tenant: &Tenant,
    ) -> Result<usize, IngestError> {
        let bytes = to_bytes(body, usize::MAX).await.expect("ingest body");
        *self.seen_ingest.lock().expect("lock") = Some(SeenIngest {
            content_type: content_type.map(str::to_owned),
            content_encoding: content_encoding.map(str::to_owned),
            tenant: tenant.clone(),
            body_len: bytes.len(),
        });
        match &self.ingest {
            IngestAnswer::Ok(rows) => Ok(*rows),
            IngestAnswer::TooLarge(message) => Err(IngestError::TooLarge(message.clone())),
            IngestAnswer::Invalid(message) => Err(IngestError::Invalid(message.clone())),
            IngestAnswer::Storage => Err(IngestError::Storage),
        }
    }

    async fn list_traces(
        &self,
        scope: &TraceScope,
        window: (i64, i64),
        cursor: Option<&str>,
    ) -> Result<TracePage, TraceReadError> {
        let (start_ms, end_ms) = window;
        *self.seen_list.lock().expect("lock") = Some(SeenList {
            scope: scope.clone(),
            start_ms,
            end_ms,
            cursor: cursor.map(str::to_owned),
        });
        read_answer(&self.list)
    }

    async fn get_trace(
        &self,
        trace_id: &str,
        _scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<Trace>, TraceReadError> {
        self.record_trace(trace_id, trace_ref);
        read_answer(&self.trace)
    }

    async fn get_span(
        &self,
        trace_id: &str,
        span_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<SpanDetail>, TraceReadError> {
        let seen = SeenSpan {
            trace_id: trace_id.to_owned(),
            span_id: span_id.to_owned(),
            scope: scope.clone(),
            trace_ref: trace_ref.to_owned(),
        };
        *self.seen_span.lock().expect("lock") = Some(seen);
        read_answer(&self.span)
    }

    async fn get_span_error(
        &self,
        target: SpanErrorTarget<'_>,
        scope: &TraceScope,
        cursor: Option<&str>,
    ) -> Result<Option<SpanErrorPage>, TraceReadError> {
        let mut slot = self.seen_error.lock().expect("lock");
        *slot = Some(SeenError {
            trace_id: target.trace_id.to_owned(),
            span_id: target.span_id.to_owned(),
            scope: scope.clone(),
            trace_ref: target.trace_ref.to_owned(),
            cursor: cursor.map(str::to_owned),
        });
        drop(slot);
        read_answer(&self.span_error)
    }

    fn query_store(&self) -> Arc<dyn TraceQueryStore> {
        Arc::<FakeQueryStore>::clone(&self.queries)
    }
}

fn read_answer<T>(answer: &ReadAnswer<T>) -> Result<T, TraceReadError>
where
    T: Clone,
{
    match answer {
        ReadAnswer::Ok(value) => Ok(value.clone()),
        ReadAnswer::InvalidCursor(message) => Err(TraceReadError::InvalidCursor(message.clone())),
        ReadAnswer::Storage => Err(TraceReadError::Storage),
    }
}

pub(super) fn application(
    receiver: Option<Arc<dyn TraceRuntime>>,
    master_key: Option<&str>,
) -> Router {
    router::router(TraceRoutesState {
        receiver,
        master_key: master_key.map(|key| SecretString::from(key.to_owned())),
        clock: Arc::new(FixedClock),
    })
}

pub(super) struct Answer {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

pub(super) fn build_request(
    method: Method,
    uri: &str,
    principal: Option<&Principal>,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::from(body.to_vec()))
        .expect("request");
    for (name, value) in headers {
        request.headers_mut().insert(
            HeaderName::try_from(*name).expect("header name"),
            HeaderValue::try_from(*value).expect("header value"),
        );
    }
    if let Some(identity) = principal {
        request.extensions_mut().insert(AuthContext {
            principal: identity.clone(),
        });
    }
    request
}

pub(super) async fn send(application: Router, request: Request<Body>) -> Answer {
    let response = application.oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    Answer {
        status,
        headers,
        body,
    }
}

pub(super) fn principal(
    roles: Vec<Role>,
    team_id: Option<&str>,
    token: Option<&str>,
    org_id: Option<&str>,
    project_id: Option<&str>,
) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: "virtual-key".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: org_id.map(|id| OrganizationIdentity {
            id: id.to_owned(),
            name: None,
        }),
        teams: team_id
            .map(|id| {
                vec![TeamIdentity {
                    id: id.to_owned(),
                    name: None,
                    role: TeamRole::Member,
                }]
            })
            .unwrap_or_default(),
        project: project_id.map(|id| ProjectIdentity {
            id: id.to_owned(),
            name: None,
        }),
        end_user: None,
        roles,
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef {
            key_id: None,
            token_id: token.map(str::to_owned),
        },
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    }
}

pub(super) fn master_principal() -> Principal {
    master_caller().principal
}

pub(super) fn body_text(answer: &Answer) -> String {
    String::from_utf8(answer.body.clone()).expect("utf8 body")
}

pub(super) fn span_row() -> SpanRow {
    SpanRow {
        timestamp: 1_000_000,
        trace_id: "trace-1".to_owned(),
        span_id: "span-1".to_owned(),
        parent_span_id: String::new(),
        trace_state: String::new(),
        span_name: "chat gpt-4o".to_owned(),
        span_kind: "SPAN_KIND_CLIENT".to_owned(),
        service_name: "demo".to_owned(),
        resource_attributes: IndexMap::from([("service.name".to_owned(), "demo".to_owned())]),
        scope_name: String::new(),
        scope_version: String::new(),
        span_attributes: IndexMap::new(),
        duration: 1_000_000,
        status_code: "STATUS_CODE_OK".to_owned(),
        status_message: String::new(),
        team_id: String::new(),
        api_key_hash: String::new(),
        observation_type: SpanType::Llm,
        agent_name: "researcher".to_owned(),
        model: "gpt-4o".to_owned(),
        litellm_request_id: "req-1".to_owned(),
        input_tokens: 3,
        output_tokens: 4,
        input: "hello".to_owned(),
        output: "world".to_owned(),
    }
}

pub(super) fn decoded_span() -> DecodedSpan {
    let normalized = NormalizedSpan {
        observation_type: SpanType::Llm,
        agent_name: "researcher".to_owned(),
        litellm_request_id: "req-1".to_owned(),
        model: "gpt-4o".to_owned(),
        input_tokens: 3,
        output_tokens: 4,
        input: "hello".to_owned(),
        output: "world".to_owned(),
    };
    let resource = IndexMap::from([("service.name".to_owned(), "demo".to_owned())]);
    let attributes = IndexMap::from([("gen_ai.response.id".to_owned(), "req-1".to_owned())]);
    DecodedSpan {
        trace_id: "trace-1".to_owned(),
        span_id: "span-1".to_owned(),
        parent_span_id: String::new(),
        trace_state: String::new(),
        name: "chat gpt-4o".to_owned(),
        kind: "SPAN_KIND_CLIENT".to_owned(),
        resource_attributes: resource,
        scope_name: String::new(),
        scope_version: String::new(),
        attributes,
        start_ns: 1_000_000,
        end_ns: 2_000_000,
        status_code: "STATUS_CODE_OK".to_owned(),
        status_message: String::new(),
        events: Vec::new(),
        normalized,
        consumed_attributes: vec!["gen_ai.response.id".to_owned()],
    }
}
