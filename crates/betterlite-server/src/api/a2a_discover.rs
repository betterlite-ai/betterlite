pub mod discovery;
pub mod source;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;
use discovery::{AgentCardDiscoveryError, DiscoveryMode};

pub const DISCOVER_PATH: &str = "/v1/a2a/discover";

#[derive(Clone)]
pub struct A2aDiscoverState {
    pub discovery: Arc<dyn DiscoverAgentCard>,
}

#[async_trait]
pub trait DiscoverAgentCard: Send + Sync {
    async fn discover(&self, request: &DiscoverAgentRequest)
    -> Result<AgentJson, DiscoveryFailure>;
}

#[derive(Debug, Error)]
pub enum DiscoveryFailure {
    #[error(transparent)]
    Card(#[from] AgentCardDiscoveryError),
    // LiteLLM turns any exception it did not expect into a 500, so the message stands alone
    #[error("{message}")]
    Unexpected { message: String },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(from = "RawDiscoverAgentRequest")]
pub struct DiscoverAgentRequest {
    pub url: String,
    pub discovery_mode: DiscoveryMode,
    pub params: BTreeMap<String, AgentJson>,
}

#[derive(Deserialize)]
struct RawDiscoverAgentRequest {
    url: String,
    #[serde(default)]
    discovery_mode: Option<DiscoveryMode>,
    #[serde(default)]
    params: Option<BTreeMap<String, AgentJson>>,
}

// An absent or null discovery_mode and absent or null params take their defaults, resolved here where the body enters
impl From<RawDiscoverAgentRequest> for DiscoverAgentRequest {
    fn from(raw: RawDiscoverAgentRequest) -> Self {
        Self {
            url: raw.url,
            discovery_mode: raw.discovery_mode.unwrap_or_default(),
            params: raw.params.unwrap_or_default(),
        }
    }
}

#[derive(Serialize)]
struct DiscoverAgentResponse {
    url: String,
    agent_card: AgentJson,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Debug, Error)]
pub enum DiscoverRequestError {
    #[error("invalid request body: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid request body at {path}: {source}")]
    Field {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid request body: {source}")]
    Trailing {
        #[source]
        source: serde_json::Error,
    },
}

impl DiscoverRequestError {
    fn decode(error: serde_path_to_error::Error<serde_json::Error>) -> Self {
        let path = error.path().to_string();
        let source = error.into_inner();
        if path == "." {
            Self::Body { source }
        } else {
            Self::Field { path, source }
        }
    }
}

#[derive(Debug, Error)]
enum DiscoverHttpError {
    #[error("Only proxy admins can discover agent cards. Your role={role}")]
    Forbidden { role: String },
    #[error("{detail}")]
    BadRequest { detail: String },
    #[error("Discovery failed: {detail}")]
    Internal { detail: String },
}

impl IntoResponse for DiscoverHttpError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(DetailBody {
            detail: self.to_string(),
        });
        (status, body).into_response()
    }
}

pub fn routes(state: A2aDiscoverState) -> Router {
    Router::new()
        .route(DISCOVER_PATH, post(discover_agent_card))
        .with_state(state)
}

async fn discover_agent_card(
    State(state): State<A2aDiscoverState>,
    Extension(context): Extension<AuthContext>,
    body: Bytes,
) -> Result<Json<DiscoverAgentResponse>, DiscoverHttpError> {
    let request = parse_request(&body).map_err(|error| DiscoverHttpError::BadRequest {
        detail: error.to_string(),
    })?;
    if !context.principal.roles.contains(&Role::PlatformAdmin) {
        return Err(DiscoverHttpError::Forbidden {
            role: role_label(&context.principal),
        });
    }
    match state.discovery.discover(&request).await {
        Ok(agent_card) => Ok(Json(DiscoverAgentResponse {
            url: request.url,
            agent_card,
        })),
        Err(DiscoveryFailure::Card(error)) => Err(DiscoverHttpError::BadRequest {
            detail: error.to_string(),
        }),
        Err(DiscoveryFailure::Unexpected { message }) => {
            Err(DiscoverHttpError::Internal { detail: message })
        }
    }
}

fn parse_request(body: &[u8]) -> Result<DiscoverAgentRequest, DiscoverRequestError> {
    let mut decoder = serde_json::Deserializer::from_slice(body);
    let request =
        serde_path_to_error::deserialize(&mut decoder).map_err(DiscoverRequestError::decode)?;
    decoder
        .end()
        .map_err(|source| DiscoverRequestError::Trailing { source })?;
    Ok(request)
}

fn role_label(principal: &Principal) -> String {
    match principal.roles.as_slice() {
        [role, ..] => role.to_string(),
        [] => "internal_user".to_owned(),
    }
}
