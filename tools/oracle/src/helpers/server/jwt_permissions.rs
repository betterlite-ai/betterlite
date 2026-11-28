use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_user_lookup::roles::RolePermissions;
use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_permissions::admin::{AdminAuthResult, AdminIdentity, check_admin_access};
use betterlite_auth::jwt_permissions::role::{
    can_rbac_role_call_model, can_rbac_role_call_route, check_rbac_role, role_permissions,
};
use betterlite_auth::jwt_permissions::scope::check_scope_based_access;
use betterlite_config::sections::auth_user_lookup::{RbacRole, RoleBasedPermissions};
use betterlite_config::sections::jwt_settings::Settings;
use betterlite_config::sections::jwt_settings::records::ScopeMapping;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    CanCallRoute,
    CanCallModel,
    CheckRbacRole,
    ScopeAccess,
    AdminAccess,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    role: Option<RbacRole>,
    #[serde(default)]
    role_permissions: Option<Vec<RoleBasedPermissions>>,
    #[serde(default)]
    enforce_rbac: bool,
    #[serde(default)]
    route: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    scope_mappings: Option<Vec<ScopeMapping>>,
    #[serde(default)]
    admin_jwt_scope: Option<String>,
    #[serde(default)]
    admin_allowed_routes: Option<Vec<String>>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_email: Option<String>,
    #[serde(default)]
    org_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    claims: Option<TypedValue>,
}

// every answer carries the same fields so a case can ignore the message text at one pointer
#[derive(Debug, Serialize)]
struct Answer {
    outcome: &'static str,
    status: Option<u16>,
    #[serde(rename = "type")]
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
    error_class: Option<&'static str>,
    message: Option<String>,
    result: Option<Value>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::CanCallRoute => {
                let permissions = permissions(input)?;
                let role = required_role(input, "can_call_route")?;
                outcome(can_rbac_role_call_route(&permissions, role, &input.route))
            }
            Function::CanCallModel => {
                let permissions = permissions(input)?;
                let role = required_role(input, "can_call_model")?;
                outcome(can_rbac_role_call_model(
                    &permissions,
                    role,
                    input.model.as_deref(),
                ))
            }
            Function::CheckRbacRole => {
                let permissions = permissions(input)?;
                outcome(check_rbac_role(
                    &settings(input),
                    &permissions,
                    input.role,
                    input.model.as_deref(),
                    &input.route,
                ))
            }
            Function::ScopeAccess => outcome(check_scope_based_access(
                input.scope_mappings.as_deref(),
                &input.scopes,
                input.model.as_deref(),
            )),
            Function::AdminAccess => admin(input),
        }
    }
}

fn permissions(input: &Input) -> Result<RolePermissions, Box<HelperError>> {
    role_permissions(input.role_permissions.as_deref()).or_value_error()
}

fn required_role(input: &Input, function: &str) -> Result<RbacRole, Box<HelperError>> {
    input
        .role
        .ok_or_else(|| format!("the {function} function needs a role"))
        .or_value_error()
}

fn settings(input: &Input) -> Settings {
    let mut settings = Settings::default();
    settings.rbac.enforce_rbac = input.enforce_rbac;
    if let Some(scope) = &input.admin_jwt_scope {
        settings.routes.admin_jwt_scope.clone_from(scope);
    }
    if let Some(routes) = &input.admin_allowed_routes {
        settings.routes.admin_allowed_routes.clone_from(routes);
    }
    if let Some(mappings) = &input.scope_mappings {
        settings.scopes.scope_mappings = Some(mappings.clone());
    }
    settings
}

fn admin(input: &Input) -> HelperResult {
    let identity = AdminIdentity {
        token: input.token.as_deref().unwrap_or_default(),
        user_id: input.user_id.as_deref(),
        user_email: input.user_email.as_deref(),
        org_id: input.org_id.as_deref(),
        agent_id: input.agent_id.as_deref(),
    };
    match check_admin_access(
        &settings(input),
        &input.scopes,
        &input.route,
        &identity,
        input.claims.as_ref(),
    ) {
        Ok(None) => emit(answer("not_admin", None)),
        Ok(Some(result)) => emit(answer("granted", Some(granted(input, &result)))),
        Err(failure) => emit(denial(&failure)),
    }
}

fn granted(input: &Input, result: &AdminAuthResult) -> Value {
    json!({
        "is_proxy_admin": result.is_proxy_admin,
        "token_preserved": input
            .token
            .as_deref()
            .is_some_and(|token| token == result.token.expose_secret()),
        "team_id": &result.team_id,
        "user_id": &result.user_id,
        "user_email": &result.user_email,
        "end_user_id": &result.end_user_id,
        "org_id": &result.org_id,
        "agent_id": &result.agent_id,
        "jwt_claims": &result.jwt_claims.json,
    })
}

fn outcome(result: Result<(), AuthFailure>) -> HelperResult {
    match result {
        Ok(()) => emit(answer("allowed", None)),
        Err(failure) => emit(denial(&failure)),
    }
}

fn answer(outcome: &'static str, result: Option<Value>) -> Answer {
    Answer {
        outcome,
        status: None,
        error_type: None,
        param: None,
        code: None,
        error_class: None,
        message: None,
        result,
    }
}

fn denial(failure: &AuthFailure) -> Answer {
    let exception = as_proxy_exception(failure).exception();
    Answer {
        outcome: "denied",
        status: Some(exception.code.parse::<u16>().unwrap_or_default()),
        error_type: Some(exception.error_type.clone()),
        param: exception.param.clone(),
        code: Some(exception.code.clone()),
        error_class: Some(class(failure)),
        message: Some(exception.message.clone()),
        result: None,
    }
}

const fn class(failure: &AuthFailure) -> &'static str {
    match failure {
        AuthFailure::ModelAccessDenied { .. } => "model_access_denied",
        AuthFailure::Http { .. } => "http",
        AuthFailure::BudgetExceeded { .. }
        | AuthFailure::Proxy { .. }
        | AuthFailure::Database(_)
        | AuthFailure::Other { .. } => "plain",
    }
}

fn emit(answer: Answer) -> HelperResult {
    Ok(serde_json::to_value(answer)?)
}
