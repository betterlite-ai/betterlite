mod runtime;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

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
use betterlite_server::auth::auth_identity::AuthContext;
use betterlite_server::tracing_routes::TraceRoutesState;
use betterlite_server::tracing_routes::access::SystemClock;
use betterlite_server::tracing_routes::router;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use tower::ServiceExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

use self::runtime::ScriptedRuntime;

const INGEST_PATH: &str = "/v1/traces";

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
    pub(crate) script: Script,
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
    pub(crate) user_role: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) token: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) org_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Script {
    #[serde(default)]
    pub(crate) decoder: Value,
    #[serde(default)]
    pub(crate) list_traces: Value,
    #[serde(default)]
    pub(crate) get_trace: Value,
    #[serde(default)]
    pub(crate) get_span: Value,
    #[serde(default)]
    pub(crate) get_span_error: Value,
    #[serde(default)]
    pub(crate) query: Value,
    #[serde(default)]
    pub(crate) query_help: Value,
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
            .map_err(|error| failure(error))?;
        runtime.block_on(answer(input))
    }
}

async fn answer(input: &Input) -> HelperResult {
    let erased: Arc<dyn betterlite_server::tracing_routes::receiver::TraceRuntime> =
        Arc::new(ScriptedRuntime {
            script: input.script.clone(),
        });
    let receiver = if input.tracing { Some(erased) } else { None };
    let state = TraceRoutesState {
        receiver,
        master_key: input.master_key.clone().map(SecretString::from),
        clock: Arc::new(SystemClock),
    };
    let mut request = build_request(input)?;
    request.extensions_mut().insert(AuthContext {
        principal: principal(&input.auth),
    });
    let response = router::router(state)
        .oneshot(request)
        .await
        .map_err(|error| failure(error))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| failure(error))?;
    if !is_ingest(input) && !status.is_success() {
        return Err(Box::new(HelperError {
            class: "HTTPException".to_owned(),
            status: Some(status.as_u16()),
            error_type: None,
            param: None,
            code: None,
            message: detail(&body),
        }));
    }
    Ok(route_value(status, &headers, &body))
}

fn is_ingest(input: &Input) -> bool {
    input.request.method == "POST" && input.request.path == INGEST_PATH
}

fn detail(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

fn route_value(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> Value {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    serde_json::json!({
        "status": status.as_u16(),
        "content_type": header_text(headers, header::CONTENT_TYPE.as_str()),
        "body": parsed,
        "body_base64": STANDARD.encode(body),
        "retry_after": header_text(headers, header::RETRY_AFTER.as_str()),
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
        .method(
            Method::from_bytes(input.request.method.as_bytes()).map_err(|error| failure(error))?,
        )
        .uri(uri_text(input));
    for (name, value) in &input.request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
        .body(Body::from(request_body(input)?))
        .map_err(|error| failure(error))
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
            .map_err(|error| failure(error)),
    }
}

fn uri_text(input: &Input) -> String {
    if input.request.query.is_empty() {
        return input.request.path.clone();
    }
    let pairs: Vec<String> = input
        .request
        .query
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    format!("{}?{}", input.request.path, pairs.join("&"))
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

fn failure<Message>(message: Message) -> Box<HelperError>
where
    Message: std::fmt::Display,
{
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
