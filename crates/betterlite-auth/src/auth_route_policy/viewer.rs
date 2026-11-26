use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;

use crate::management_routes::{
    GLOBAL_SPEND_TRACKING_ROUTES, admin_viewer_routes, management_routes,
};
use crate::route_match::classifier::is_llm_api_route;
use crate::route_match::templates::{RouteValue, check_route_access};

use super::denial;
use super::viewer_tables::{
    EARLY_BLOCKED_KEY_SUFFIXES, EARLY_BLOCKED_ROUTES, LATE_BLOCKED_KEY_SUFFIX,
    LATE_BLOCKED_WRITE_ROUTES, SAFE_HTTP_METHODS,
};

const USER_UPDATE: &str = "/user/update";
const USER_EMAIL: &str = "user_email";
const PASSWORD_CHANGE: &str = "/user/password/change";
const SESSION_LOGOUT: &str = "/session/logout";
const DEFAULT_METHOD: &str = "GET";
const KEY_ROUTE_PREFIX: &str = "/key/";

// RouteChecks._check_proxy_admin_viewer_access: inference routes cost money, safe verbs read, and every
// other write needs an explicit allowance.
pub fn check_access(
    route: &str,
    method: Option<&str>,
    body: &JsonObject,
) -> Result<(), Box<ProxyException>> {
    if is_llm_api_route(&RouteValue::Text(route)) {
        return Err(denial::viewer_llm_route(route));
    }
    if management_route_denied(route, body)? {
        return Err(denial::viewer_route(route));
    }
    if SAFE_HTTP_METHODS.contains(&method.unwrap_or(DEFAULT_METHOD)) {
        return Ok(());
    }
    if route == USER_UPDATE {
        return check_update_fields(route, body);
    }
    if route == PASSWORD_CHANGE || route == SESSION_LOGOUT {
        return Ok(());
    }
    if late_denied(route) {
        return Err(denial::viewer_route(route));
    }
    if check_route_access(route, &admin_viewer_routes())
        || check_route_access(route, GLOBAL_SPEND_TRACKING_ROUTES)
    {
        return Ok(());
    }
    Err(denial::viewer_route(route))
}

// the management branch that runs before the method decides: email-only updates and the early deny table
fn management_route_denied(route: &str, body: &JsonObject) -> Result<bool, Box<ProxyException>> {
    if !check_route_access(route, &management_routes()) {
        return Ok(false);
    }
    if route == USER_UPDATE {
        check_update_fields(route, body)?;
        return Ok(false);
    }
    Ok(early_denied(route))
}

fn early_denied(route: &str) -> bool {
    if check_route_access(route, EARLY_BLOCKED_ROUTES) {
        return true;
    }
    blocked_key_suffix(route, &EARLY_BLOCKED_KEY_SUFFIXES)
}

fn late_denied(route: &str) -> bool {
    if check_route_access(route, LATE_BLOCKED_WRITE_ROUTES) {
        return true;
    }
    blocked_key_suffix(route, &[LATE_BLOCKED_KEY_SUFFIX])
}

fn check_update_fields(route: &str, body: &JsonObject) -> Result<(), Box<ProxyException>> {
    for param in body.keys() {
        if param != USER_EMAIL {
            return Err(denial::viewer_update_param(route, param));
        }
    }
    Ok(())
}

fn blocked_key_suffix(route: &str, suffixes: &[&str]) -> bool {
    if !route.starts_with(KEY_ROUTE_PREFIX) {
        return false;
    }
    suffixes.iter().any(|suffix| route.ends_with(suffix))
}
