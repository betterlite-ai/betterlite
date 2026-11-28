use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

const AUTH_ERROR: &str = "auth_error";
const PARAM_NONE: &str = "None";
const UNAUTHORIZED: i64 = 401;
pub(crate) const FORBIDDEN: i64 = 403;

// betterlite's own wording: the same status, type, param and code LiteLLM answers, the message names the caller
#[must_use]
pub(crate) fn admin_only_route(
    route: &str,
    role: &str,
    masked_user_id: &str,
) -> Box<ProxyException> {
    exception(
        format!(
            "Only a proxy admin can call {route}. Your role={role}. Your user_id={masked_user_id}"
        ),
        UNAUTHORIZED,
    )
}

#[must_use]
pub(crate) fn invalid_token() -> Box<ProxyException> {
    exception(
        "Invalid proxy server token passed. valid_token=None.".to_owned(),
        UNAUTHORIZED,
    )
}

#[must_use]
pub(crate) fn other_user_info(
    route: &str,
    requested_user_id: &str,
    own_user_id: Option<&str>,
) -> Box<ProxyException> {
    let own = own_user_id.unwrap_or(PARAM_NONE);
    exception(
        format!(
            "the key may not read another user's info on {route}. user_id={requested_user_id}, key's user_id={own}"
        ),
        FORBIDDEN,
    )
}

#[must_use]
pub(crate) fn custom_admin_only(route: &str) -> Box<ProxyException> {
    exception(
        format!("{route} is one of general_settings.admin_only_routes"),
        FORBIDDEN,
    )
}

#[must_use]
pub(crate) fn viewer_llm_route(route: &str) -> Box<ProxyException> {
    exception(
        format!("an admin viewer may not call the inference route {route}"),
        FORBIDDEN,
    )
}

#[must_use]
pub(crate) fn viewer_update_param(route: &str, param: &str) -> Box<ProxyException> {
    exception(
        format!("an admin viewer may change only user_email on {route}, not {param}"),
        FORBIDDEN,
    )
}

pub(crate) fn exception(message: String, status: i64) -> Box<ProxyException> {
    let init = ProxyExceptionInit {
        message,
        error_type: AUTH_ERROR.to_owned(),
        param: Some(PARAM_NONE.to_owned()),
        code: ExceptionCode::Status(status),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    };
    Box::new(ProxyException::new(init))
}
