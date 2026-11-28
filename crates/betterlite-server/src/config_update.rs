pub mod body;
mod merge;
mod route;
pub mod router_settings;
pub mod store;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_router::routing_groups::error::RoutingGroupError;
use thiserror::Error;

use self::store::ConfigUpdateStore;
use crate::audit_events::callbacks::AuditTaskSpawner;
use crate::audit_events::sink::AuditEventSink;
use crate::deployment_sync_port::DeploymentSyncPort;
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::state::ProxyConfigState;
use crate::search_tool_routes::body::BodyError;

pub type SettingsMap = BTreeMap<String, JsonValue>;

const NOT_ADMIN: &str = "Only proxy admins can update config";

// The settings stores the route guards on, the database it writes, and the audit sink and deployment sync its caller owns
#[derive(Clone)]
pub struct ConfigUpdateState {
    pub config: Arc<ProxyConfigState>,
    pub database: Option<Arc<dyn ConfigUpdateStore>>,
    pub audit: Arc<AuditEventSink>,
    pub deployments: Arc<dyn DeploymentSyncPort>,
    pub spawner: Arc<dyn AuditTaskSpawner>,
}

#[derive(Debug, Error)]
pub enum ConfigUpdateError {
    #[error("Only proxy admins can update config")]
    NotAdmin,
    #[error("Unsupported router settings: {names} are not valid router settings")]
    UnsupportedRouterSettings { names: String },
    #[error(transparent)]
    RoutingGroups(#[from] RoutingGroupError),
    #[error("routing_groups must be a list")]
    RoutingGroupsList,
    #[error("routing_groups entries must be objects")]
    RoutingGroupsEntry,
    #[error("No DB Connected")]
    NoDatabase,
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error(transparent)]
    ConfigState(#[from] ConfigStateError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

impl ConfigUpdateError {
    const fn status(&self) -> StatusCode {
        match self {
            Self::NotAdmin => StatusCode::FORBIDDEN,
            Self::UnsupportedRouterSettings { .. }
            | Self::RoutingGroups(_)
            | Self::RoutingGroupsList
            | Self::RoutingGroupsEntry
            | Self::NoDatabase
            | Self::Body(_)
            | Self::ConfigState(_)
            | Self::Encode(_) => StatusCode::BAD_REQUEST,
        }
    }

    // an HTTPException keeps its detail; every other exception is prefixed, as update_config's except block does
    fn detail(&self) -> String {
        match self {
            Self::NotAdmin => NOT_ADMIN.to_owned(),
            Self::UnsupportedRouterSettings { .. }
            | Self::RoutingGroups(_)
            | Self::RoutingGroupsList
            | Self::RoutingGroupsEntry => self.to_string(),
            Self::NoDatabase | Self::Body(_) | Self::ConfigState(_) | Self::Encode(_) => {
                format!("Authentication Error, {self}")
            }
        }
    }
}

impl IntoResponse for ConfigUpdateError {
    fn into_response(self) -> Response {
        if let Self::Body(error) = self {
            return error.into_response();
        }
        auth_error(self.status(), &self.detail())
    }
}

// ProxyException(type=auth_error, param=getattr(e, "param", "None"), code=status): param is the text None
fn auth_error(status: StatusCode, message: &str) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: message.to_owned(),
            error_type: WireField::Text("auth_error".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text(status.as_u16().to_string()),
        },
    };
    (status, Json(body)).into_response()
}

pub fn routes(state: ConfigUpdateState) -> Router {
    Router::new()
        .route("/config/update", post(route::update_config))
        .with_state(state)
}
