use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use crate::config_overlay_port::ConfigOverlayPortError;
use crate::plugin_routes::error::PluginError;
use crate::proxy_config_state::error::ConfigStateError;
use crate::settings_services::error::SettingsUpdateError;
use crate::tracing_routes::router::unauthenticated;

use super::ports::ConfigFieldRowError;

pub const DB_NOT_CONNECTED: &str = "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys";
pub const NOT_ALLOWED: &str = "Admin-only endpoint. Not allowed to access.";

// every refusal LiteLLM raises with HTTPException(detail={"error": ...}), plus the errors behind its unhandled handler
#[derive(Debug, Error)]
pub enum ConfigFieldError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("{detail}")]
    BadRequest { detail: String },
    #[error("the config database is not reachable")]
    Unavailable,
    #[error(transparent)]
    Save(ConfigStateError),
    #[error("the configuration could not be updated")]
    Internal,
}

#[derive(Serialize)]
struct DetailBody<'text> {
    detail: DetailText<'text>,
}

#[derive(Serialize)]
struct DetailText<'text> {
    error: &'text str,
}

impl IntoResponse for ConfigFieldError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::BadRequest { detail } => bad_request(&detail),
            Self::Unavailable => {
                SettingsUpdateError::from(ConfigOverlayPortError::DatabaseUnavailable)
                    .into_response()
            }
            Self::Save(source) => SettingsUpdateError::from(source).into_response(),
            Self::Internal => server_error(),
        }
    }
}

impl From<ConfigStateError> for ConfigFieldError {
    fn from(source: ConfigStateError) -> Self {
        Self::Save(source)
    }
}

impl From<ConfigOverlayPortError> for ConfigFieldError {
    fn from(source: ConfigOverlayPortError) -> Self {
        match source {
            ConfigOverlayPortError::DatabaseUnavailable => Self::Unavailable,
            ConfigOverlayPortError::Reload | ConfigOverlayPortError::PassThrough => Self::Internal,
        }
    }
}

impl From<ConfigFieldRowError> for ConfigFieldError {
    fn from(source: ConfigFieldRowError) -> Self {
        match source {
            ConfigFieldRowError::Unavailable => Self::Unavailable,
            ConfigFieldRowError::Read => Self::Internal,
        }
    }
}

impl From<PluginError> for ConfigFieldError {
    fn from(_source: PluginError) -> Self {
        Self::Internal
    }
}

// LiteLLM's FastAPI HTTPException(status_code=400, detail={"error": text})
fn bad_request(detail: &str) -> Response {
    let body = DetailBody {
        detail: DetailText { error: detail },
    };
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}

// otel_unhandled_exception_handler, as SettingsUpdateError already projects it
fn server_error() -> Response {
    let source = ConfigStateError::InvalidStoredSection {
        section: "general_settings",
    };
    SettingsUpdateError::from(source).into_response()
}
