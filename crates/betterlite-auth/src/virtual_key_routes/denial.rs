use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

const AUTH_ERROR: &str = "auth_error";
const PARAM_NONE: &str = "None";
const FORBIDDEN: i64 = 403;

#[must_use]
pub fn auth_pass_through_denied(route: &str) -> Box<ProxyException> {
    forbidden(format!(
        "Key/team not allowed to access passthrough route {route}. Configure `allowed_passthrough_routes` on the team or key."
    ))
}

#[must_use]
pub fn password_reset_denied() -> Box<ProxyException> {
    forbidden(
        "This account's password must be changed before the session can be used: it was either found in a known data breach or set by an admin. Change it via POST /user/password/change (UI: /ui/change-password), then log in again."
            .to_owned(),
    )
}

#[must_use]
pub fn virtual_route_denied(route: &str, allowed_routes: &[&str]) -> Box<ProxyException> {
    forbidden(format!(
        "Virtual key is not allowed to call this route. Only allowed to call routes: {}. Tried to call route: {route}",
        route_list_repr(allowed_routes)
    ))
}

fn forbidden(message: String) -> Box<ProxyException> {
    let init = ProxyExceptionInit {
        message,
        error_type: AUTH_ERROR.to_owned(),
        param: Some(PARAM_NONE.to_owned()),
        code: ExceptionCode::Status(FORBIDDEN),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    };
    Box::new(ProxyException::new(init))
}

// LiteLLM interpolates the Python repr of the key's list into the message
fn route_list_repr(routes: &[&str]) -> String {
    let mut rendered = String::from("[");
    for (index, route) in routes.iter().enumerate() {
        if index > 0 {
            rendered.push_str(", ");
        }
        rendered.push('\'');
        rendered.push_str(&route.replace('\\', "\\\\").replace('\'', "\\'"));
        rendered.push('\'');
    }
    rendered.push(']');
    rendered
}
