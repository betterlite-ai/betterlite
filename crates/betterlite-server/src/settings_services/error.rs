use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::database_availability::chain::Chain;
use betterlite_db::database_availability::projection::service_unavailable_error;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;
use thiserror::Error;

use super::config::SettingsConfigError;
use crate::config_overlay_port::ConfigOverlayPortError;
use crate::proxy_config_state::error::ConfigStateError;

const INTERNAL_MESSAGE: &str = "Internal server error";
const INTERNAL_TYPE: &str = "internal_server_error";

pub const STORAGE_DISABLED: &str =
    "Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature.";

// A pinned schema descriptor that cannot be projected: LiteLLM raises while reading the pydantic schema and the route answers its unhandled 500
#[derive(Debug, Error)]
pub enum SettingsSchemaError {
    #[error("the pinned schema is not a JSON schema")]
    InvalidSchema {
        #[source]
        source: serde_json::Error,
    },
    #[error("the pinned schema has no properties")]
    MissingProperties,
    #[error("the pinned schema's {section} are not schema nodes")]
    InvalidDefinitions {
        section: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Error)]
pub enum SettingsUpdateError {
    #[error("Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature.")]
    StorageDisabled,
    #[error(transparent)]
    Overlay(#[from] ConfigOverlayPortError),
    #[error(transparent)]
    Config(#[from] SettingsConfigError),
    #[error("the config could not be saved")]
    Save(#[from] ConfigStateError),
}

#[derive(Serialize)]
struct DetailBody<Detail> {
    detail: Detail,
}

#[derive(Serialize)]
struct ErrorDetail {
    error: &'static str,
}

// otel_unhandled_exception_handler's body: no param or code fields
#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

impl IntoResponse for SettingsUpdateError {
    fn into_response(self) -> Response {
        match &self {
            Self::StorageDisabled => storage_disabled(),
            Self::Overlay(ConfigOverlayPortError::DatabaseUnavailable) => database_unavailable(),
            // a save that hits a disconnect or pool timeout answers the pinned unhandled handler's no_db_connection 503 too
            Self::Save(ConfigStateError::Database { source }) => {
                if is_native_transport_error(source) {
                    database_unavailable()
                } else {
                    unhandled_error(&self)
                }
            }
            Self::Overlay(ConfigOverlayPortError::Reload | ConfigOverlayPortError::PassThrough)
            | Self::Config(_)
            | Self::Save(
                ConfigStateError::InvalidSetting { .. }
                | ConfigStateError::InvalidStoredSection { .. }
                | ConfigStateError::MissingTeamId { .. }
                | ConfigStateError::WriteConfigFile { .. }
                | ConfigStateError::SerializeConfig { .. }
                | ConfigStateError::Encode { .. }
                | ConfigStateError::Decode { .. }
                | ConfigStateError::EnvironmentValue(_),
            ) => unhandled_error(&self),
            // _raise_config_owned's HTTPException detail, answered with save_config's 400
            Self::Save(ConfigStateError::ConfigOwned(detail)) => {
                (StatusCode::BAD_REQUEST, Json(DetailBody { detail })).into_response()
            }
        }
    }
}

// the pinned HTTPException detail for a proxy that does not store its config in the database
fn storage_disabled() -> Response {
    let body = DetailBody {
        detail: ErrorDetail {
            error: STORAGE_DISABLED,
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

// B1217's projection, whose status is the 503 constant
fn database_unavailable() -> Response {
    let unavailable = service_unavailable_error(&Chain::default());
    let body = ErrorResponse {
        error: ErrorObject {
            message: unavailable.message,
            error_type: WireField::Text(unavailable.error_type.to_owned()),
            param: WireField::Text(unavailable.param.to_owned()),
            code: WireField::Text(unavailable.code.to_owned()),
        },
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

fn unhandled_error(error: &SettingsUpdateError) -> Response {
    tracing::error!(error = %error, "the settings update failed");
    let body = InternalBody {
        error: InternalMessage {
            message: INTERNAL_MESSAGE,
            error_type: INTERNAL_TYPE,
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}
