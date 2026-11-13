use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, OrganizationIdentity, Principal, ProjectIdentity, SourceKey,
    TeamIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role, TeamRole};
use betterlite_logging::otlp_normalize::SpanRow;
use betterlite_logging::trace_types::{SpanDetail, SpanErrorPage, Trace, TracePage, TraceScope};
use betterlite_server::tracing_routes::TraceRoutesState;
use betterlite_server::tracing_routes::access::{TraceClock, Tenant};
use betterlite_server::tracing_routes::query::{QueryRunError, QueryScope, TraceQueryStore};
use betterlite_server::tracing_routes::receiver::{IngestError, TraceReadError, TraceRuntime};
use betterlite_server::tracing_routes::router;
use betterlite_tracing::storage::trace_store::SpanErrorTarget;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

const DEFAULT_DECODER_REJECTION: &str = "OTLP payload rejected";
const DEFAULT_READ_FAILURE: &str = "the trace read failed";
const DEFAULT_QUERY_FAILURE: &str = "the trace query failed";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) request: RequestInput,
    #[serde(default)]
    pub(crate) auth: AuthInput,
    #[serde(default = "default_tracing")]
    pub(crate) tracing: bool,
    #[serde(default)]
    pub(crate) master_key: Option<String>,
    #[serde(default)]
    pub(crate) script: Box<Script>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    pub(crate) method: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: Option<BodyInput>,
    #[serde(default)]
    pub(crate) query: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyInput {
    pub(crate) encoding: Encoding,
    pub(crate) value: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Encoding {
    Utf8,
    Base64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthInput {
    #[serde(default)]
    pub(crate) user_role: Option<String>,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    #[serde(default)]
    pub(crate) token: Option<String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) org_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Script {
    #[serde(default)]
    pub(crate) decoder: DecoderScript,
    #[serde(default)]
    pub(crate) now_ms: Option<i64>,
    #[serde(default)]
    pub(crate) list_traces: ReadScript<TracePage>,
    #[serde(default)]
    pub(crate) get_trace: ReadScript<Trace>,
    #[serde(default)]
    pub(crate) get_span: ReadScript<SpanDetail>,
    #[serde(default)]
    pub(crate) get_span_error: ReadScript<SpanErrorPage>,
    #[serde(default)]
    pub(crate) query: QueryScript,
    #[serde(default)]
    pub(crate) query_help: QueryScript,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecoderScript {
    #[serde(default)]
    pub(crate) kind: DecoderKind,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) rows: Vec<SpanRow>,
}

impl Default for DecoderScript {
    fn default() -> Self {
        Self {
            kind: DecoderKind::Ok,
            message: None,
            rows: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecoderKind {
    #[default]
    Ok,
    TooLarge,
    Overflow,
    Malformed,
    Overloaded,
}

#[derive(Debug, Clone)]
pub(crate) struct ReadScript<Payload> {
    pub(crate) value: Option<Payload>,
    pub(crate) error: Option<ErrorScript>,
}

impl<Payload> Default for ReadScript<Payload> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
        }
    }
}

impl<'de, Payload> Deserialize<'de> for ReadScript<Payload>
where
    Payload: Deserialize<'de>,
{
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let raw = RawReadScript::<Payload>::deserialize(deserializer)?;
        Ok(Self {
            value: raw.value,
            error: raw.error,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReadScript<Value> {
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    error: Option<ErrorScript>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueryScript {
    #[serde(default)]
    pub(crate) value: Option<String>,
    #[serde(default)]
    pub(crate) error: Option<ErrorScript>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorScript {
    pub(crate) kind: ErrorKind,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    InvalidCursor,
    Storage,
    Invalid,
    Unavailable,
}

fn default_tracing() -> bool {
    true
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(answer(input))
    }
}

struct FixedClock {
    now_ms: i64,
}

impl TraceClock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.now_ms
    }
}

struct ScriptedRuntime {
    script: Script,
    seen: Arc<Mutex<Option<Value>>>,
    queries: Arc<ScriptedQueries>,
}

#[async_trait]
impl TraceRuntime for ScriptedRuntime {
    async fn ingest(
        &self,
        body: Body,
        _content_type: Option<&str>,
        _content_encoding: Option<&str>,
        tenant: &Tenant,
    ) -> Result<usize, IngestError> {
        let payload = to_bytes(body, usize::MAX)
            .await
            .map_err(IngestError::BodyRead)?;
        let kind = self.script.decoder.kind;
        let message = self.decoder_message();
        match kind {
            DecoderKind::TooLarge | DecoderKind::Overflow => Err(IngestError::TooLarge(message)),
            DecoderKind::Malformed => Err(IngestError::Invalid(message)),
            DecoderKind::Overloaded => Err(IngestError::Overloaded),
            DecoderKind::Ok => {
                let _consumed = payload.len();
                let mut rows = self.script.decoder.rows.clone();
                tenant.stamp_rows(&mut rows);
                self.record(json!({
                    "kind": "ingest",
                    "tenant": tenant_json(tenant),
                    "rows": rows,
                }));
                Ok(rows.len())
            }
        }
    }

    async fn list_traces(
        &self,
        scope: &TraceScope,
        window: (i64, i64),
        cursor: Option<&str>,
    ) -> Result<TracePage, TraceReadError> {
        self.record(json!({
            "kind": "list_traces",
            "scope": scope,
            "start_ms": window.0,
            "end_ms": window.1,
            "cursor": cursor,
        }));
        read_answer(&self.script.list_traces)
    }

    async fn get_trace(
        &self,
        trace_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<Trace>, TraceReadError> {
        self.record(json!({
            "kind": "get_trace",
            "trace_id": trace_id,
            "trace_ref": trace_ref,
            "scope": scope,
        }));
        read_answer(&self.script.get_trace)
    }

    async fn get_span(
        &self,
        trace_id: &str,
        span_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<SpanDetail>, TraceReadError> {
        self.record(json!({
            "kind": "get_span",
            "trace_id": trace_id,
            "span_id": span_id,
            "trace_ref": trace_ref,
            "scope": scope,
        }));
        read_answer(&self.script.get_span)
    }

    async fn get_span_error(
        &self,
        target: SpanErrorTarget<'_>,
        scope: &TraceScope,
        cursor: Option<&str>,
    ) -> Result<Option<SpanErrorPage>, TraceReadError> {
        self.record(json!({
            "kind": "get_span_error",
            "trace_id": target.trace_id,
            "span_id": target.span_id,
            "trace_ref": target.trace_ref,
            "cursor": cursor,
            "scope": scope,
        }));
        read_answer(&self.script.get_span_error)
    }

    fn query_store(&self) -> Arc<dyn TraceQueryStore> {
        let store: Arc<dyn TraceQueryStore> = Arc::clone(&self.queries);
        store
    }
}

impl ScriptedRuntime {
    fn decoder_message(&self) -> String {
        self.script
            .decoder
            .message
            .clone()
            .unwrap_or_else(|| DEFAULT_DECODER_REJECTION.to_owned())
    }

    fn record(&self, value: Value) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(value);
        }
    }
}

struct ScriptedQueries {
    sql: QueryScript,
    help: QueryScript,
    seen: Arc<Mutex<Option<Value>>>,
}

#[async_trait]
impl TraceQueryStore for ScriptedQueries {
    async fn query_sql(
        &self,
        sql: &str,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        self.record(json!({
            "kind": "query",
            "sql": sql,
            "scope": scope,
            "secret": secret.expose_secret(),
        }));
        query_answer(&self.sql)
    }

    async fn query_help(
        &self,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        self.record(json!({
            "kind": "query_help",
            "scope": scope,
            "secret": secret.expose_secret(),
        }));
        query_answer(&self.help)
    }
}

impl ScriptedQueries {
    fn record(&self, value: Value) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(value);
        }
    }
}

fn read_answer<Payload>(script: &ReadScript<Payload>) -> Result<Payload, TraceReadError>
where
    Payload: Clone,
{
    match (&script.value, &script.error) {
        (Some(value), _) => Ok(value.clone()),
        (None, Some(error)) => Err(read_error(error)),
        (None, None) => Err(TraceReadError::Storage),
    }
}

fn read_error(error: &ErrorScript) -> TraceReadError {
    let message = error
        .message
        .clone()
        .unwrap_or_else(|| DEFAULT_READ_FAILURE.to_owned());
    match error.kind {
        ErrorKind::InvalidCursor => TraceReadError::InvalidCursor(message),
        ErrorKind::Storage | ErrorKind::Invalid | ErrorKind::Unavailable => {
            TraceReadError::Storage
        }
    }
}

fn query_answer(script: &QueryScript) -> Result<String, QueryRunError> {
    match (&script.value, &script.error) {
        (Some(value), _) => Ok(value.clone()),
        (None, Some(error)) => {
            let message = error
                .message
                .clone()
                .unwrap_or_else(|| DEFAULT_QUERY_FAILURE.to_owned());
            match error.kind {
                ErrorKind::Unavailable => Err(QueryRunError::Unavailable(message)),
                ErrorKind::Invalid | ErrorKind::InvalidCursor | ErrorKind::Storage => {
                    Err(QueryRunError::Invalid(message))
                }
            }
        }
        (None, None) => Err(QueryRunError::Invalid(DEFAULT_QUERY_FAILURE.to_owned())),
    }
}

fn tenant_json(tenant: &Tenant) -> Value {
    json!({
        "team_id": tenant.team_id,
        "api_key_hash": tenant.api_key_hash,
        "org_id": tenant.org_id,
    })
}

fn principal(auth: &AuthInput) -> Principal {
    let roles = match auth.user_role.as_deref() {
        Some("proxy_admin") => vec![Role::PlatformAdmin],
        Some("proxy_admin_view_only") => vec![Role::PlatformViewer],
        Some(_unrecognized) | None => Vec::new(),
    };
    Principal {
        principal_type: PrincipalType::Human,
        subject: "virtual-key".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: auth.org_id.as_ref().map(|id| OrganizationIdentity {
            id: id.clone(),
            name: None,
        }),
        teams: auth
            .team_id
            .as_ref()
            .map(|id| {
                vec![TeamIdentity {
                    id: id.clone(),
                    name: None,
                    role: TeamRole::Member,
                }]
            })
            .unwrap_or_default(),
        project: auth.project_id.as_ref().map(|id| ProjectIdentity {
            id: id.clone(),
            name: None,
        }),
        end_user: None,
        roles,
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef {
            key_id: None,
            token_id: auth.token.clone(),
        },
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    }
}

async fn answer(input: &Input) -> HelperResult {
    let seen = Arc::new(Mutex::new(None));
    let query_seen = Arc::new(Mutex::new(None));
    let queries = Arc::new(ScriptedQueries {
        sql: input.script.query.clone(),
        help: input.script.query_help.clone(),
        seen: Arc::clone(&query_seen),
    });
    let runtime = Arc::new(ScriptedRuntime {
        script: input.script.as_ref().clone(),
        seen: Arc::clone(&seen),
        queries,
    });
    let erased: Arc<dyn TraceRuntime> = runtime;
    let receiver: Option<Arc<dyn TraceRuntime>> = if input.tracing {
        Some(erased)
    } else {
        None
    };
    let state = TraceRoutesState {
        receiver,
        master_key: input.master_key.clone().map(SecretString::from),
        clock: Arc::new(FixedClock {
            now_ms: input.script.now_ms.unwrap_or(0),
        }),
    };
    let request = build_request(input)?;
    let response = router::router(state)
        .oneshot(request)
        .await
        .map_err(|error| failure("RuntimeError", error))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| failure("RuntimeError", error))?;
    let received = if is_query_route(input) {
        query_seen.lock().ok().and_then(|seen| seen.clone())
    } else {
        seen.lock().ok().and_then(|seen| seen.clone())
    };
    if let Some(error) = route_error(input, status, &body) {
        return Err(Box::new(error));
    }
    Ok(route_value(status, &headers, &body, received))
}

fn is_query_route(input: &Input) -> bool {
    input.request.path.starts_with("/v1/traces/query")
}

fn route_error(input: &Input, status: StatusCode, body: &[u8]) -> Option<HelperError> {
    let ingest = input.request.method == "POST" && input.request.path == "/v1/traces";
    if ingest || status.is_success() {
        return None;
    }
    Some(HelperError {
        class: "HTTPException".to_owned(),
        status: Some(status.as_u16()),
        error_type: None,
        param: None,
        code: None,
        message: detail_text(body),
    })
}

fn detail_text(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("detail").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

fn route_value(
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
    received: Option<Value>,
) -> Value {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    json!({
        "status": status.as_u16(),
        "content_type": header_text(headers, header::CONTENT_TYPE.as_str()),
        "body": parsed,
        "body_base64": STANDARD.encode(body),
        "retry_after": header_text(headers, header::RETRY_AFTER.as_str()),
        "received": received.unwrap_or(Value::Null),
    })
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn build_request(input: &Input) -> Result<Request<Body>, Box<HelperError>> {
    let mut builder = Request::builder()
        .method(Method::from_bytes(input.request.method.as_bytes()).map_err(|error| {
            failure("ValueError", error)
        })?)
        .uri(uri_text(input));
    for (name, value) in &input.request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let body = request_body(input)?;
    builder.body(Body::from(body)).map_err(|error| failure("ValueError", error))
}

fn request_body(input: &Input) -> Result<Bytes, Box<HelperError>> {
    let Some(body) = input.request.body.as_ref() else {
        return Ok(Bytes::new());
    };
    match body.encoding {
        Encoding::Utf8 => Ok(Bytes::copy_from_slice(body.value.as_bytes())),
        Encoding::Base64 => STANDARD
            .decode(&body.value)
            .map(Bytes::from)
            .map_err(|error| failure("ValueError", error)),
    }
}

fn uri_text(input: &Input) -> String {
    if input.request.query.is_empty() {
        return input.request.path.clone();
    }
    let encoded: Vec<String> = input
        .request
        .query
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    format!("{}?{}", input.request.path, encoded.join("&"))
}

fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

mod tests;
