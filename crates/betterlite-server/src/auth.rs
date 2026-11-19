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

use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_auth::auth_request_models::MODEL_ROUTING_HEADER_NAME;
use betterlite_auth::auth_request_models::catalog::{EmptyCatalog, RequestModelResolver};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::auth_model_patterns::{
    ModelPatternStageInput, max_recurse_depth_for_request, rejection_response, rule_for_request,
};
use crate::auth::auth_request_models::{CoreDecoders, RequestModelStageInput, resolve};
use crate::auth::auth_resource_grants::{
    ResourceGrantStageInput, database_for_request, denial_response,
    rule_for_request as grant_rule_for_request,
};
use crate::auth::auth_tag_budgets::TagBudgetStageInput;
use crate::error_body::{AuthenticationFailure, authentication_error};
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
    let resolver = request
        .extensions()
        .get::<Arc<RequestModelResolver>>()
        .map_or_else(default_resolver, Arc::clone);
    let mut authenticated: Option<AuthContext> = None;
    for backend in INSTALLED_STAGES {
        let input = match stage_input(*backend, &request, authenticated.as_ref(), &resolver).await {
            Ok(input) => input,
            Err(failure) => return stage_failure_response(&failure),
        };
        match dispatch(*backend, &input, master_key) {
            Ok(context) => authenticated = Some(context),
            Err(failure) => return stage_failure_response(&failure),
        }
    }
    if let Some(context) = authenticated {
        request.extensions_mut().insert(context);
    }
    next.run(request).await
}

fn stage_failure_response(failure: &StageFailure) -> Response {
    match failure {
        StageFailure::Authentication(failure) => authentication_error(failure),
        StageFailure::ModelAccess(rejection) => rejection_response(rejection),
        StageFailure::ResourceGrants(exception) | StageFailure::TagBudgets(exception) => {
            denial_response(exception)
        }
    }
}

async fn stage_input(
    backend: AuthBackend,
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    resolver: &Arc<RequestModelResolver>,
) -> Result<StageInput, StageFailure> {
    match backend {
        AuthBackend::MasterKey => Ok(StageInput::MasterKey(extract_key(request.headers()))),
        AuthBackend::AuthRequestModels | AuthBackend::AuthModelPatterns => {
            model_patterns_stage(backend, request, authenticated, resolver)
        }
        AuthBackend::AuthResourceGrants => {
            let context = authenticated
                .cloned()
                .ok_or(AuthenticationFailure::Missing)?;
            let stage = ResourceGrantStageInput {
                context,
                rule: grant_rule_for_request(request),
                database: database_for_request(request),
            };
            Ok(StageInput::AuthResourceGrants(Box::new(stage)))
        }
        AuthBackend::AuthTagBudgets => tag_budget_stage_input(request, authenticated).await,
    }
}

fn model_patterns_stage(
    backend: AuthBackend,
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    resolver: &Arc<RequestModelResolver>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let stage = model_stage_input(request, context.clone(), resolver);
    if backend == AuthBackend::AuthRequestModels {
        return Ok(StageInput::AuthRequestModels(Box::new(stage)));
    }
    let requested = match resolve(&stage) {
        Ok(requested) => requested,
        Err(error) => {
            tracing::debug!(error = %error, "the request model stage failed");
            return Err(StageFailure::from(AuthenticationFailure::Missing));
        }
    };
    let rule = rule_for_request(request);
    let depth = max_recurse_depth_for_request(request);
    Ok(StageInput::AuthModelPatterns(Box::new(
        ModelPatternStageInput {
            context,
            requested,
            rule,
            max_recurse_depth: depth,
        },
    )))
}

// the batch reads are asynchronous, so this stage resolves its rows while its input is assembled
async fn tag_budget_stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let stage = TagBudgetStageInput {
        context,
        rule: auth_tag_budgets::rule_for_request(request),
        database: auth_tag_budgets::database_for_request(request),
    };
    auth_tag_budgets::prefetch(&stage)
        .await
        .map_err(StageFailure::TagBudgets)?;
    Ok(StageInput::AuthTagBudgets(Box::new(stage)))
}
fn default_resolver() -> Arc<RequestModelResolver> {
    Arc::new(RequestModelResolver {
        catalog: Arc::new(EmptyCatalog),
        decoders: Arc::new(CoreDecoders),
    })
}
fn model_stage_input(
    request: &Request<Body>,
    context: AuthContext,
    resolver: &Arc<RequestModelResolver>,
) -> RequestModelStageInput {
    let route = request.uri().path().to_owned();
    let headers = headers_json(request.headers());
    let query = query_json(request.uri().query());
    let body = JsonObject::new();
    RequestModelStageInput {
        context,
        route,
        body,
        headers: Some(headers),
        query: Some(query),
        endpoint_marker: None,
        path_params: BTreeSet::new(),
        team_id: None,
        resolver: Arc::clone(resolver),
    }
}
fn headers_json(headers: &HeaderMap) -> JsonObject {
    let mut object = JsonObject::new();
    if let Some(value) = headers.get(MODEL_ROUTING_HEADER_NAME) {
        // LiteLLM decodes header bytes as latin-1, and only this header feeds the model candidates
        let text: String = value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect();
        object.insert(MODEL_ROUTING_HEADER_NAME.to_owned(), Json::Text(text));
    }
    object
}
const MODEL_QUERY_KEYS: [&str; 2] = ["model", "target_model_names"];

fn query_json(query: Option<&str>) -> JsonObject {
    let mut object = JsonObject::new();
    for (name, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        // LiteLLM reads these two query keys into the model candidates and nothing else
        if MODEL_QUERY_KEYS.contains(&name.to_lowercase().as_str()) {
            object.insert(name.into_owned(), Json::Text(value.into_owned()));
        }
    }
    object
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
