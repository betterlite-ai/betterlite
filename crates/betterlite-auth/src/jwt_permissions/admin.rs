use betterlite_config::sections::jwt_settings::Settings;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;

use crate::auth_errors::conversion::AuthFailure;
use crate::auth_route_policy::jwt::allowed_routes_check;
use crate::jwt_claims::TypedValue;
use crate::route_match::groups::get_actual_routes;
use crate::user_roles::UserRole;

// the verified identity authorize_jwt passes into check_admin_access
pub struct AdminIdentity<'identity> {
    pub token: &'identity str,
    pub user_id: Option<&'identity str>,
    pub user_email: Option<&'identity str>,
    pub org_id: Option<&'identity str>,
    pub agent_id: Option<&'identity str>,
}

// JWTAuthBuilderResult fields the admin branch owns: the JWT's identity with no team or end-user object
#[derive(Clone, Debug)]
pub struct AdminAuthResult {
    pub is_proxy_admin: bool,
    pub token: SecretString,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub end_user_id: Option<String>,
    pub org_id: Option<String>,
    pub agent_id: Option<String>,
    pub jwt_claims: TypedValue,
}

// is_admin gates the branch; an admin-scope token still meets the configured admin_allowed_routes
pub fn check_admin_access(
    settings: &Settings,
    scopes: &[String],
    route: &str,
    identity: &AdminIdentity<'_>,
    claims: Option<&TypedValue>,
) -> Result<Option<AdminAuthResult>, Box<AuthFailure>> {
    if !scopes
        .iter()
        .any(|scope| scope == &settings.routes.admin_jwt_scope)
    {
        return Ok(None);
    }
    if !allowed_routes_check(
        UserRole::ProxyAdmin,
        route,
        &settings.routes.admin_allowed_routes,
        None,
    ) {
        return Err(route_denial(settings, route));
    }
    // `claims or {}`: an empty typed map becomes the wire document, since json!({}) names the banned Map type
    let jwt_claims = claims.map_or_else(
        || TypedValue {
            json: serde_json::json!(JsonObject::new()),
        },
        Clone::clone,
    );
    Ok(Some(admin_result(identity, jwt_claims)))
}

// the plain Exception check_admin_access raises, which B757 turns into the 401 auth error
fn route_denial(settings: &Settings, route: &str) -> Box<AuthFailure> {
    let configured = settings
        .routes
        .admin_allowed_routes
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    Box::new(AuthFailure::Other {
        message: format!(
            "Admin not allowed to access this route. Route={route}, Allowed Routes={:?}",
            get_actual_routes(&configured)
        ),
        param: None,
    })
}

// JWTAuthBuilderResult's admin branch: the token, identity and claims with no team, end-user or membership object
fn admin_result(identity: &AdminIdentity<'_>, jwt_claims: TypedValue) -> AdminAuthResult {
    AdminAuthResult {
        is_proxy_admin: true,
        token: SecretString::from(identity.token.to_owned()),
        team_id: None,
        user_id: identity.user_id.map(str::to_owned),
        user_email: identity.user_email.map(str::to_owned),
        end_user_id: None,
        org_id: identity.org_id.map(str::to_owned),
        agent_id: identity.agent_id.map(str::to_owned),
        jwt_claims,
    }
}
