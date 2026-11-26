use std::collections::BTreeMap;

use betterlite_auth::auth_route_policy::admission::{RoutePolicyInput, is_api_route_allowed};
use betterlite_auth::auth_route_policy::jwt::allowed_routes_check;
use betterlite_auth::auth_route_policy::mask::mask_user_id;
use betterlite_auth::auth_route_policy::viewer::check_access;
use betterlite_auth::pass_through_registry::PassThroughRegistry;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    RouteAllowed,
    ViewerAccess,
    MaskedUserId,
    JwtAllowedRoutes,
}

// the fields RouteChecks reads from the request, the caller's key, the caller's user row and the route config
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    route: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    user_id_query: Option<String>,
    #[serde(default)]
    body: JsonObject,
    #[serde(default)]
    token: Option<KeyFields>,
    #[serde(default)]
    user: Option<UserFields>,
    #[serde(default)]
    admin_only_routes: Vec<String>,
    #[serde(default)]
    team_allowed_routes: Option<Vec<String>>,
    #[serde(default)]
    admin_allowed_routes: Vec<String>,
    #[serde(default)]
    role: Option<UserRole>,
    #[serde(default)]
    masked: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct KeyFields {
    user_id: Option<String>,
    allowed_routes: Option<Vec<JsonData>>,
    metadata: Option<BTreeMap<String, JsonData>>,
    team_metadata: Option<BTreeMap<String, JsonData>>,
    permissions: Option<BTreeMap<String, JsonData>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserFields {
    user_id: String,
    #[serde(default)]
    user_role: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::RouteAllowed => route_allowed(input),
            Function::ViewerAccess => Ok(viewer_access(input)),
            Function::MaskedUserId => Ok(json!({"masked": mask_user_id(&input.masked)})),
            Function::JwtAllowedRoutes => jwt_allowed_routes(input),
        }
    }
}

fn route_allowed(input: &Input) -> HelperResult {
    let token = caller_key(input.token.as_ref())?;
    let user = user_row(input.user.as_ref())?;
    let registry = PassThroughRegistry {
        root_path: String::new(),
        registrations: Vec::new(),
    };
    let method = input.method.as_deref().map(str::to_uppercase);
    let policy = RoutePolicyInput {
        route: &input.route,
        method: method.as_deref(),
        user_id_query: input.user_id_query.as_deref(),
        body: &input.body,
        token: token.as_ref(),
        user: user.as_ref(),
        registry: &registry,
        admin_only_routes: &input.admin_only_routes,
        team_allowed_routes: input.team_allowed_routes.as_deref().unwrap_or_default(),
    };
    Ok(answer(is_api_route_allowed(&policy)))
}

fn viewer_access(input: &Input) -> Value {
    let method = input.method.as_deref().map(str::to_uppercase);
    answer(check_access(&input.route, method.as_deref(), &input.body))
}

fn jwt_allowed_routes(input: &Input) -> HelperResult {
    let role = input
        .role
        .ok_or("jwt_allowed_routes needs a role")
        .or_value_error()?;
    let allowed = allowed_routes_check(
        role,
        &input.route,
        &input.admin_allowed_routes,
        input.team_allowed_routes.as_deref(),
    );
    Ok(json!({"allowed": allowed, "error": Value::Null}))
}

fn caller_key(source: Option<&KeyFields>) -> Result<Option<UserAPIKeyAuth>, Box<HelperError>> {
    let Some(fields) = source else {
        return Ok(None);
    };
    let mut dict = BTreeMap::new();
    if let Some(user_id) = &fields.user_id {
        dict.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
    }
    if let Some(routes) = &fields.allowed_routes {
        dict.insert("allowed_routes".to_owned(), JsonData::Array(routes.clone()));
    }
    for (name, value) in [
        ("metadata", fields.metadata.as_ref()),
        ("team_metadata", fields.team_metadata.as_ref()),
        ("permissions", fields.permissions.as_ref()),
    ] {
        if let Some(entries) = value {
            dict.insert(name.to_owned(), JsonData::Object(entries.clone()));
        }
    }
    let token = UserAPIKeyAuth::construct(AuthInput::Dict(dict)).or_value_error()?;
    Ok(Some(token))
}

fn user_row(source: Option<&UserFields>) -> Result<Option<LiteLLMUserTable>, Box<HelperError>> {
    let Some(fields) = source else {
        return Ok(None);
    };
    let row = json!({
        "user_id": fields.user_id.clone(),
        "user_role": fields.user_role.clone(),
    });
    Ok(Some(serde_json::from_value(row)?))
}

fn answer(result: Result<(), Box<ProxyException>>) -> Value {
    match result {
        Ok(()) => json!({"allowed": true, "error": Value::Null}),
        Err(exception) => json!({"allowed": false, "error": denial(exception.as_ref())}),
    }
}

fn denial(exception: &ProxyException) -> Value {
    json!({
        "status": exception.code.parse::<u16>().unwrap_or_default(),
        "error_type": exception.error_type.as_str(),
        "param": &exception.param,
        "code": exception.code.as_str(),
    })
}
