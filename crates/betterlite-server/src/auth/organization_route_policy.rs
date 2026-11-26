use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_route_patterns::canonical::{
    RequestRouteSource, ScopeFields, get_request_route,
};
use betterlite_auth::management_routes::ORG_ADMIN_ONLY_ROUTES;
use betterlite_auth::organization_route_policy::membership::RouteUser;
use betterlite_auth::organization_route_policy::role_check::{
    TEAM_CREATE_ROUTE, check_organization_role,
};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::request_body_safety::request_object;
use crate::auth::route_match::decoded_route_path;
use crate::error_body::AuthenticationFailure;

mod tests;

// what the auth service resolved before this stage: the request's user row, or None when the request carries no user
#[derive(Clone, Debug, Default)]
pub struct OrganizationRouteRule {
    pub user: Option<RouteUser>,
}

#[derive(Clone, Debug)]
pub struct OrganizationPolicyStageInput {
    pub context: AuthContext,
    pub user: Option<RouteUser>,
    pub route: String,
    pub body: JsonObject,
}

// the authentication service inserts the rule once it resolved the user; without one the stage is inert
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
        .get::<Arc<OrganizationRouteRule>>()
        .map_or_else(|| Arc::new(OrganizationRouteRule::default()), Arc::clone);
    let route = request_route(request);
    let reads_organization =
        ORG_ADMIN_ONLY_ROUTES.contains(&route.as_str()) || route == TEAM_CREATE_ROUTE;
    // only the selected-organization checks read the body, so no other route pays for parsing it
    let parsed = if rule.user.is_none() || !reads_organization {
        JsonObject::new()
    } else {
        request_body(request, body)
    };
    Ok(StageInput::OrganizationRoutePolicy(Box::new(
        OrganizationPolicyStageInput {
            context,
            user: rule.user.clone(),
            route,
            body: parsed,
        },
    )))
}

pub(crate) fn run_stage(
    stage: &OrganizationPolicyStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    check_organization_role(stage.user.as_ref(), &stage.route, &stage.body)
        .map_err(|error| Box::new(error.into_proxy_exception()))?;
    Ok(stage.context.clone())
}

// LiteLLM hands the role checks the request's parsed body, so the auth reader's object is what the policy sees
fn request_body(request: &Request<Body>, body: Option<&Bytes>) -> JsonObject {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let bytes = body.map_or(&[][..], |body| &body[..]);
    request_object(bytes, content_type).unwrap_or_default()
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
