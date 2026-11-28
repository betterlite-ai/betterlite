use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_route_patterns::canonical::{
    RequestRouteSource, ScopeFields, get_request_route,
};
use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_permissions::admin::{AdminIdentity, check_admin_access};
use betterlite_auth::jwt_permissions::role::{
    RolePermissionSetupError, check_rbac_role, role_permissions,
};
use betterlite_auth::jwt_permissions::scope::check_scope_based_access;
use betterlite_config::sections::auth_user_lookup::{RbacRole, RoleBasedPermissions};
use betterlite_config::sections::jwt_settings;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::request_body_safety::request_object;
use crate::auth::route_match::decoded_route_path;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// the verified claims JWT authentication records on the request; without one the master key and virtual keys leave the stage alone
#[derive(Clone, Debug, Default)]
pub struct JwtPermissionRule {
    pub role: Option<RbacRole>,
    pub scopes: Vec<String>,
    pub claims: Option<TypedValue>,
    pub token: Option<SecretString>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub org_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct JwtPermissionStageInput {
    pub context: AuthContext,
    pub rule: Option<Arc<JwtPermissionRule>>,
    pub route: String,
    pub model: Option<String>,
    pub settings: jwt_settings::Settings,
    pub role_permissions: Option<Vec<RoleBasedPermissions>>,
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
        .get::<Arc<JwtPermissionRule>>()
        .cloned();
    let (settings, role_permissions) = configured(request);
    let stage = JwtPermissionStageInput {
        context,
        rule,
        route: request_route(request),
        model: request_model(request, body),
        settings,
        role_permissions,
    };
    Ok(StageInput::JwtPermissions(Box::new(stage)))
}

// check_rbac_role, then the enforced scope mappings, then the admin branch: LiteLLM's authorize_jwt order
pub(crate) fn run_stage(
    stage: &JwtPermissionStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    let Some(rule) = stage.rule.as_deref() else {
        return Ok(stage.context.clone());
    };
    authorize(stage, rule)
        .map_err(|failure| Box::new(as_proxy_exception(&failure).exception().clone()))?;
    Ok(stage.context.clone())
}

fn authorize(
    stage: &JwtPermissionStageInput,
    rule: &JwtPermissionRule,
) -> Result<(), Box<AuthFailure>> {
    let permissions = role_permissions(stage.role_permissions.as_deref()).map_err(setup_failure)?;
    check_rbac_role(
        &stage.settings,
        &permissions,
        rule.role,
        stage.model.as_deref(),
        &stage.route,
    )?;
    if stage.settings.scopes.enforce_scope_based_access {
        check_scope_based_access(
            stage.settings.scopes.scope_mappings.as_deref(),
            &rule.scopes,
            stage.model.as_deref(),
        )?;
    }
    let identity = AdminIdentity {
        token: rule
            .token
            .as_ref()
            .map_or("", |token| token.expose_secret()),
        user_id: rule.user_id.as_deref(),
        user_email: rule.user_email.as_deref(),
        org_id: rule.org_id.as_deref(),
        agent_id: rule.agent_id.as_deref(),
    };
    // B053 turns the builder result into JWTAuthBuilderResult; the stage enforces the route gate before any of it is used
    let _admin_result = check_admin_access(
        &stage.settings,
        &rule.scopes,
        &stage.route,
        &identity,
        rule.claims.as_ref(),
    )?;
    Ok(())
}

// the config section already validated this list, so a failed conversion is a wiring error that fails closed
fn setup_failure(error: RolePermissionSetupError) -> Box<AuthFailure> {
    tracing::error!(%error, "general_settings.role_permissions could not be read");
    Box::new(AuthFailure::Other {
        message: String::from("the JWT permission configuration could not be read"),
        param: None,
    })
}

fn configured(
    request: &Request<Body>,
) -> (jwt_settings::Settings, Option<Vec<RoleBasedPermissions>>) {
    request.extensions().get::<Arc<ServerState>>().map_or_else(
        || (jwt_settings::Settings::default(), None),
        |server| {
            (
                server.jwt_permissions.settings.clone(),
                server.jwt_permissions.role_permissions.clone(),
            )
        },
    )
}

// request_data.get("model"): the body's declared string field, and anything else means no model
fn request_model(request: &Request<Body>, body: Option<&Bytes>) -> Option<String> {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let object = request_object(body?, content_type)?;
    match object.get("model") {
        Some(Json::Text(model)) => Some(model.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
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
