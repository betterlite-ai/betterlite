pub mod auth_admission;
pub mod auth_backend_dispatch;
pub mod auth_cached_sessions;
pub mod auth_common_checks;
pub mod auth_context_sources;
pub mod auth_end_users;
pub mod auth_errors;
pub mod auth_headers;
pub mod auth_identity;
pub mod auth_key_budgets;
pub mod auth_key_lookup;
pub mod auth_key_validation;
pub mod auth_model_budgets;
pub mod auth_model_grants;
pub mod auth_model_lists;
pub mod auth_model_patterns;
pub mod auth_org_lookup;
pub mod auth_policy_support;
pub mod auth_pricing;
pub mod auth_request_models;
pub mod auth_request_policy;
pub mod auth_resource_grants;
pub mod auth_route_patterns;
pub mod auth_route_policy;
pub mod auth_tag_budgets;
pub mod auth_tenant_budgets;
pub mod budget_admission_handoff;
pub mod central_auth_checks;
pub mod current_spend;
pub mod end_user_identity;
pub mod entity_model_access;
pub mod identity_store;
pub mod jwt_db_teams;
pub mod jwt_key_resolution;
pub mod jwt_keys;
pub mod jwt_permissions;
pub mod jwt_provisioning;
pub mod jwt_team_authorization;
pub mod jwt_team_selection;
pub mod jwt_verification;
pub mod key_context;
pub mod master_database_auth;
pub mod model_grant_intersection;
pub mod oauth_tokens;
pub mod organization_route_policy;
pub mod request_body_safety;
pub mod route_match;
pub mod team_budget_admission;
pub mod trusted_network;
pub mod virtual_key_routes;

use crate::auth::auth_identity::{INSTALLED_STAGES, StageInput, dispatch};
use crate::error_body::authentication_error;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use regex::Regex;
use secrecy::SecretString;
pub(crate) async fn authenticate(
    State(master_key_state): State<Option<SecretString>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if matches!(
        request.uri().path(),
        "/health/liveliness" | "/health/liveness" | "/health/readiness"
    ) {
        return next.run(request).await;
    }
    let Some(master_key) = master_key_state.as_ref() else {
        return next.run(request).await;
    };
    let presented_key = extract_key(request.headers());
    for backend in INSTALLED_STAGES {
        let input = StageInput::MasterKey(presented_key.clone());
        match dispatch(*backend, &input, master_key) {
            Ok(context) => {
                request.extensions_mut().insert(context);
            }
            Err(failure) => return authentication_error(&failure),
        }
    }
    next.run(request).await
}
fn extract_key(headers: &HeaderMap) -> Option<SecretString> {
    for name in [
        "x-litellm-api-key",
        header::AUTHORIZATION.as_str(),
        "api-key",
        "x-api-key",
        "x-goog-api-key",
        "ocp-apim-subscription-key",
    ] {
        let Some(value) = headers.get(name) else {
            continue;
        };
        if let Some(key) = parse_key_header(name, value.as_bytes()) {
            return Some(key);
        }
    }
    None
}
pub fn parse_key_header(name: &str, header_bytes: &[u8]) -> Option<SecretString> {
    if header_bytes.is_empty() {
        return None;
    }
    let header_value: String = header_bytes.iter().map(|byte| char::from(*byte)).collect();
    let token = if name == header::AUTHORIZATION.as_str() {
        strip_prefix(&header_value).unwrap_or_default()
    } else if name == "x-litellm-api-key" {
        strip_prefix(&header_value).unwrap_or(header_value)
    } else {
        header_value
    };
    Some(SecretString::from(token))
}
fn strip_prefix(value: &str) -> Option<String> {
    for prefix in ["Bearer ", "Basic ", "bearer "] {
        if value.starts_with(prefix) {
            return Some(value.replace(prefix, ""));
        }
    }
    value
        .starts_with("AWS4-HMAC-SHA256")
        .then(|| aws_credential(value))
        .flatten()
}
fn aws_credential(value: &str) -> Option<String> {
    let bearer = Regex::new(r"Credential=Bearer\s+([^/\s,]+)").ok()?;
    let plain = Regex::new(r"Credential=([^/\s,]+)").ok()?;
    let captures = bearer.captures(value).or_else(|| plain.captures(value))?;
    captures.get(1).map(|token| token.as_str().to_owned())
}
