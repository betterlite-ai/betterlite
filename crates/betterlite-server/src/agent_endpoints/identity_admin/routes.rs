use std::future::Future;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_auth::agent_identity::error::AgentIdentityError;
use betterlite_auth::identity::{AgentIdentityFailure, ManagedAgentIdentityStatus};
use betterlite_core::agents::AgentResponse;

use super::error::{Detail, IdentityAdminError};
use super::issuers::{IdentitySettings, trusted_agent_issuers};
// ast-grep-ignore: no-long-names
use super::permission::check_agent_management_permission as require_agent_admin;
use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

mod tests;

const PROVIDERS_PATH: &str = "/v1/agents/identity/providers";
const STATUS_PATH: &str = "/v1/agents/{agent_id}/identity";

// The server backs this with B4773's AgentIdentityStore, whose agent method answers the same way
pub trait IdentityStatusReads: Send + Sync {
    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send;
}

#[derive(Clone)]
struct IdentityAdminState<Reads> {
    settings: IdentitySettings,
    reads: Reads,
}

pub fn agent_identity_routes<Reads>(settings: IdentitySettings, reads: Reads) -> Router
where
    Reads: IdentityStatusReads + Clone + Send + Sync + 'static,
{
    Router::new()
        .route(PROVIDERS_PATH, get(get_agent_identity_providers::<Reads>))
        .route(STATUS_PATH, get(get_agent_identity_status::<Reads>))
        .with_state(IdentityAdminState { settings, reads })
}

async fn get_agent_identity_providers<Reads>(
    State(state): State<IdentityAdminState<Reads>>,
    caller: Option<Extension<AuthContext>>,
) -> Response
where
    Reads: IdentityStatusReads + Send + Sync,
{
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let Some(key) = context.principal.source_key.0 else {
        return unauthenticated();
    };
    match require_agent_admin(&key) {
        Ok(()) => Json(trusted_agent_issuers(&state.settings)).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn get_agent_identity_status<Reads>(
    State(state): State<IdentityAdminState<Reads>>,
    Path(agent_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Response
where
    Reads: IdentityStatusReads + Send + Sync,
{
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let Some(key) = context.principal.source_key.0 else {
        return unauthenticated();
    };
    if let Err(error) = require_agent_admin(&key) {
        return error.into_response();
    }
    match state.reads.agent(&agent_id).await {
        Err(failure) => IdentityAdminError::from(AgentIdentityError::from(failure)).into_response(),
        Ok(None) => IdentityAdminError {
            status: 404,
            detail: Detail::Text(String::from("Agent not found")),
        }
        .into_response(),
        Ok(Some(agent)) => Json(status_body(agent)).into_response(),
    }
}

fn status_body(agent: AgentResponse) -> ManagedAgentIdentityStatus {
    let last_authenticated_at = agent
        .identity
        .as_ref()
        .and_then(|identity| identity.last_authenticated_at);
    ManagedAgentIdentityStatus {
        identity: agent.identity,
        identity_managed: agent.identity_managed,
        enabled: agent.enabled,
        execution_mode: agent.execution_mode,
        last_authenticated_at,
    }
}
