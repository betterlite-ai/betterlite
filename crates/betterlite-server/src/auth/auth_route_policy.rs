use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_route_patterns::canonical::{
    RequestRouteSource, ScopeFields, get_request_route,
};
use betterlite_auth::auth_route_policy::admission::{RoutePolicyInput, is_api_route_allowed};
use betterlite_auth::auxiliary_routes::INFO_ROUTES;
use betterlite_auth::pass_through_registry::PassThroughRegistry;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::RouteValue;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::request_body_safety::request_object;
use crate::auth::route_match::decoded_route_path;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// the key the auth service authenticated and its owner's live row; without one the master key leaves the stage without a policy
#[derive(Clone, Debug, Default)]
pub struct AuthRoutePolicyRule {
    pub token: Option<UserAPIKeyAuth>,
    pub user: Option<LiteLLMUserTable>,
}

#[derive(Clone, Debug)]
pub struct RoutePolicyStageInput {
    pub context: AuthContext,
    pub token: Option<UserAPIKeyAuth>,
    pub user: Option<LiteLLMUserTable>,
    pub route: String,
    pub method: Option<String>,
    pub user_id_query: Option<String>,
    pub body: JsonObject,
    pub admin_only_routes: Vec<String>,
    pub team_allowed_routes: Vec<String>,
    pub registry: PassThroughRegistry,
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let rule = request
        .extensions()
        .get::<Arc<AuthRoutePolicyRule>>()
        .cloned()
        .unwrap_or_default();
    let (admin_only_routes, team_allowed_routes, registry) = server_routes(request);
    let route = request_route(request);
    // the inference and info branches answer before the policy reads a body, so only the rest pay for parsing it
    let reads_body = rule.user.is_some()
        && !is_llm_api_route(&RouteValue::Text(&route))
        && !INFO_ROUTES.contains(&route.as_str());
    let parsed = if reads_body {
        request_body(request, body)
    } else {
        JsonObject::new()
    };
    let stage = RoutePolicyStageInput {
        context,
        token: rule.token.clone(),
        user: rule.user.clone(),
        route,
        method: Some(request.method().as_str().to_uppercase()),
        user_id_query: query_user_id(request.uri().query()),
        body: parsed,
        admin_only_routes,
        team_allowed_routes,
        registry,
    };
    Ok(StageInput::AuthRoutePolicy(Box::new(stage)))
}

// RouteChecks.non_proxy_admin_allowed_routes_check once the auth chain resolved the caller's key and row
pub(crate) fn run_stage(stage: &RoutePolicyStageInput) -> Result<AuthContext, StageFailure> {
    let token = stage
        .context
        .principal
        .source_key
        .0
        .as_ref()
        .or(stage.token.as_ref());
    let Some(caller) = token else {
        return Ok(stage.context.clone());
    };
    let input = RoutePolicyInput {
        route: &stage.route,
        method: stage.method.as_deref(),
        user_id_query: stage.user_id_query.as_deref(),
        body: &stage.body,
        token: Some(caller),
        user: stage.user.as_ref(),
        registry: &stage.registry,
        admin_only_routes: &stage.admin_only_routes,
        team_allowed_routes: &stage.team_allowed_routes,
    };
    is_api_route_allowed(&input).map_err(StageFailure::AuthRoutePolicy)?;
    Ok(stage.context.clone())
}

// the registrations and JWT team routes the virtual-key route stage reads, and the configured admin-only list
fn server_routes(request: &Request<Body>) -> (Vec<String>, Vec<String>, PassThroughRegistry) {
    request.extensions().get::<Arc<ServerState>>().map_or_else(
        || {
            (
                Vec::new(),
                Vec::new(),
                PassThroughRegistry {
                    root_path: String::new(),
                    registrations: Vec::new(),
                },
            )
        },
        |server| {
            (
                server.auth_route_policy.admin_only_routes.clone(),
                server.virtual_key_routes.team_allowed_routes.clone(),
                server.virtual_key_routes.registry_snapshot(),
            )
        },
    )
}

// LiteLLM hands the role checks the request's parsed body, so the auth reader's object is what the policy sees
fn request_body(request: &Request<Body>, body: Option<&Bytes>) -> JsonObject {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Some(bytes) = body else {
        return JsonObject::new();
    };
    request_object(bytes, content_type).unwrap_or_default()
}

// Starlette's query_params.get: the last value of a repeated name wins
fn query_user_id(query: Option<&str>) -> Option<String> {
    let mut user_id = None;
    for (name, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        if name == "user_id" {
            user_id = Some(value.into_owned());
        }
    }
    user_id
}

// get_request_route over the dispatched path, decoded as uvicorn decodes the request line
fn request_route(request: &Request<Body>) -> String {
    let raw = request.uri().path();
    let decoded = decoded_route_path(raw);
    let path = decoded.as_deref().unwrap_or(raw);
    get_request_route(&RequestRouteSource {
        scope: Ok(Some(ScopeFields {
            path: Some(path),
            app_root_path: None,
            root_path: None,
        })),
        url_path: path,
    })
}
