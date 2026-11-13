use std::sync::{Arc, Mutex};

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
use betterlite_server::tracing_routes::router;
use secrecy::SecretString;
use serde_json::Value;
use tower::ServiceExt as _;

use crate::helpers::{HelperError, HelperResult};

use super::inputs::{AuthInput, Encoding, Input};
use super::runtime::{FixedClock, ScriptedQueries, ScriptedRuntime};

pub(super) async fn answer(input: &Input) -> HelperResult {
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
    let erased: Arc<dyn betterlite_server::tracing_routes::receiver::TraceRuntime> = runtime;
    let receiver = if input.tracing { Some(erased) } else { None };
    let state = TraceRoutesState {
        receiver,
        master_key: input.master_key.clone().map(SecretString::from),
        clock: Arc::new(FixedClock {
            now_ms: input.script.now_ms.unwrap_or(0),
        }),
    };
    let mut request = build_request(input)?;
    request.extensions_mut().insert(AuthContext {
        principal: principal(&input.auth),
    });
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
        .and_then(|value| {
            value
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

fn route_value(
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
    received: Option<Value>,
) -> Value {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    serde_json::json!({
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
        .method(
            Method::from_bytes(input.request.method.as_bytes())
                .map_err(|error| failure("ValueError", error))?,
        )
        .uri(uri_text(input));
    for (name, value) in &input.request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let body = request_body(input)?;
    builder
        .body(Body::from(body))
        .map_err(|error| failure("ValueError", error))
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

pub(super) fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
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
