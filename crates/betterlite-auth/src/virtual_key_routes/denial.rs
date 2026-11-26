use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

const AUTH_ERROR: &str = "auth_error";
const PARAM_NONE: &str = "None";
const FORBIDDEN: i64 = 403;

// betterlite's own wording: the same status, type, param and code LiteLLM answers, the message names what to do
#[must_use]
pub fn auth_pass_through_denied(route: &str) -> Box<ProxyException> {
    forbidden(format!(
        "the key or team may not use the pass-through route {route}; set allowed_passthrough_routes on the key or the team"
    ))
}

#[must_use]
pub fn password_reset_denied() -> Box<ProxyException> {
    forbidden(
        "this account must change its password before using the proxy: a breach or an admin set it. Change it with POST /user/password/change (UI: /ui/change-password), then sign in again."
            .to_owned(),
    )
}

#[must_use]
pub fn virtual_route_denied(route: &str, allowed_routes: &[&str]) -> Box<ProxyException> {
    forbidden(format!(
        "the key may not call {route}; its allowed routes are {}",
        allowed_routes_text(allowed_routes)
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

// the allowed entries in order, without quoting, and "none" when the list holds no path
fn allowed_routes_text(routes: &[&str]) -> String {
    if routes.is_empty() {
        "none".to_owned()
    } else {
        routes.join(", ")
    }
}
