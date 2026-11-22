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
use super::permission::{
    check_agent_management_permission, check_agent_management_role, key_has_role,
};
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::caller_identity;
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

// the prologue both routes share: an API-key caller is checked as LiteLLM checks the key, and a master-key or JWT caller by the role on its principal
fn admitted_caller(caller: Option<Extension<AuthContext>>) -> Result<(), Response> {
    let Some(Extension(context)) = caller else {
        return Err(unauthenticated());
    };
    let checked = match context.principal.source_key.0.as_ref() {
        Some(key) if key_has_role(key) => check_agent_management_permission(key),
        Some(_) | None => match caller_identity(&context.principal) {
            Ok(identity) => check_agent_management_role(identity.user_role.as_deref()),
            Err(_error) => Err(IdentityAdminError {
                status: 500,
                detail: Detail::Text(String::from("the caller's role could not be read")),
            }),
        },
    };
    match checked {
        Ok(()) => Ok(()),
        Err(error) => Err(error.into_response()),
    }
}

async fn get_agent_identity_providers<Reads>(
    State(state): State<IdentityAdminState<Reads>>,
    caller: Option<Extension<AuthContext>>,
) -> Response
where
    Reads: IdentityStatusReads + Send + Sync,
{
    if let Err(refusal) = admitted_caller(caller) {
        return refusal;
    }
    Json(trusted_agent_issuers(&state.settings)).into_response()
}

async fn get_agent_identity_status<Reads>(
    State(state): State<IdentityAdminState<Reads>>,
    Path(agent_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Response
where
    Reads: IdentityStatusReads + Send + Sync,
{
    if let Err(refusal) = admitted_caller(caller) {
        return refusal;
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
