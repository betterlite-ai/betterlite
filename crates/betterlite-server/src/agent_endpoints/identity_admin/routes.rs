use std::future::Future;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_auth::agent_identity::error::AgentIdentityError;
use betterlite_auth::agent_policy::{AgentExecutionMode, AgentResponse as PolicyAgent};
use betterlite_auth::identity::{
    ActiveState, AgentIdentityBinding, AgentIdentityFailure, ExecutionMode,
    ManagedAgentIdentityStatus,
};

use super::error::{Detail, IdentityAdminError};
use super::issuers::{IdentitySettings, trusted_agent_issuers};
use super::permission::check_agent_management_role;
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
    ) -> impl Future<Output = Result<Option<PolicyAgent>, AgentIdentityFailure>> + Send;
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

// the prologue both routes share: the authenticated caller, its role the way plugin_routes reads it, then the admin check
fn caller_refusal(caller: Option<Extension<AuthContext>>) -> Option<Response> {
    let Some(Extension(context)) = caller else {
        return Some(unauthenticated());
    };
    let checked = match caller_identity(&context.principal) {
        Ok(identity) => check_agent_management_role(identity.user_role.as_deref()),
        Err(_error) => Err(IdentityAdminError {
            status: 500,
            detail: Detail::Text(String::from("the caller's role could not be read")),
        }),
    };
    let error = checked.err()?;
    Some(error.into_response())
}

async fn get_agent_identity_providers<Reads>(
    State(state): State<IdentityAdminState<Reads>>,
    caller: Option<Extension<AuthContext>>,
) -> Response
where
    Reads: IdentityStatusReads + Clone + Send + Sync + 'static,
{
    if let Some(refusal) = caller_refusal(caller) {
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
    Reads: IdentityStatusReads + Clone + Send + Sync + 'static,
{
    if let Some(refusal) = caller_refusal(caller) {
        return refusal;
    }
    match state.reads.agent(&agent_id).await {
        Err(failure) => IdentityAdminError::from(AgentIdentityError::from(failure)).into_response(),
        Ok(None) => IdentityAdminError {
            status: 404,
            detail: Detail::Text(String::from("Agent not found")),
        }
        .into_response(),
        Ok(Some(agent)) => match status_body(agent) {
            Ok(body) => Json(body).into_response(),
            Err(error) => error.into_response(),
        },
    }
}

// LiteLLM copies the store row into the status, so the binding is read into the response record through its wire form
fn status_body(agent: PolicyAgent) -> Result<ManagedAgentIdentityStatus, IdentityAdminError> {
    // a stored binding the response record cannot hold, such as a tenant that is not a UUID, is an unreadable policy
    let identity = agent
        .identity
        .map(|binding| {
            let wire = serde_json::to_value(binding).map_err(|_error| policy_unreadable())?;
            serde_json::from_value::<AgentIdentityBinding>(wire)
                .map_err(|_error| policy_unreadable())
        })
        .transpose()?;
    let last_authenticated_at = identity
        .as_ref()
        .and_then(|binding| binding.last_authenticated_at);
    Ok(ManagedAgentIdentityStatus {
        identity,
        identity_managed: agent.identity_managed,
        enabled: ActiveState(agent.enabled.0),
        execution_mode: execution_mode(agent.execution_mode),
        last_authenticated_at,
    })
}

fn policy_unreadable() -> IdentityAdminError {
    IdentityAdminError {
        status: 503,
        detail: Detail::Text(String::from("Agent policy could not be loaded")),
    }
}

fn execution_mode(mode: AgentExecutionMode) -> ExecutionMode {
    match mode {
        AgentExecutionMode::Autonomous => ExecutionMode::Autonomous,
        AgentExecutionMode::Delegated => ExecutionMode::Delegated,
        AgentExecutionMode::Both => ExecutionMode::Both,
    }
}
