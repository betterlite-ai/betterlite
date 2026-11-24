use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};

use super::error::TextDetail;

// _SPEND_REPORT_SCOPE_COLUMNS: only these three columns may select a scoped report, and the value is always bound as $3
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeColumn {
    ApiKey,
    User,
    TeamId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScopeError {
    Forbidden(&'static str),
    Missing(&'static str),
}

impl ScopeError {
    pub(super) fn respond(self) -> Response {
        match self {
            Self::Forbidden(scope_name) => {
                let detail =
                    format!("Not authorized to view spend for a {scope_name} other than your own");
                (StatusCode::FORBIDDEN, Json(TextDetail { detail: &detail })).into_response()
            }
            Self::Missing(scope_name) => {
                let detail = format!(
                    "No {scope_name} associated with this API key; pass a {scope_name} query param"
                );
                (
                    StatusCode::BAD_REQUEST,
                    Json(TextDetail { detail: &detail }),
                )
                    .into_response()
            }
        }
    }
}

// _resolve_spend_report_scope: a falsey request falls back to the caller, whose null value is a 400
pub(super) fn resolve_scope(
    requested: Option<&str>,
    caller_value: Option<&str>,
    admin_view: bool,
    scope_name: &'static str,
) -> Result<String, ScopeError> {
    if let Some(value) = requested.filter(|value| !value.is_empty()) {
        if caller_value.is_none_or(|own| own != value) && !admin_view {
            return Err(ScopeError::Forbidden(scope_name));
        }
        return Ok(value.to_owned());
    }
    let Some(own) = caller_value else {
        return Err(ScopeError::Missing(scope_name));
    };
    Ok(own.to_owned())
}
