mod tests;

use axum::http::StatusCode;
use betterlite_logging::analytics::python::repr;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{detail}")]
pub(crate) struct DiscoveryFailure {
    pub(crate) status: StatusCode,
    pub(crate) detail: String,
}

pub(crate) fn not_oauth_protected(name: &str) -> DiscoveryFailure {
    DiscoveryFailure {
        status: StatusCode::NOT_FOUND,
        detail: format!("MCP server '{name}' is not an OAuth-protected resource"),
    }
}

pub(crate) fn upstream_failure(name: &str, reason: &str) -> DiscoveryFailure {
    let quoted = repr(&name).unwrap_or_default();
    DiscoveryFailure {
        status: StatusCode::BAD_GATEWAY,
        detail: format!("{reason} for MCP server {quoted}"),
    }
}
