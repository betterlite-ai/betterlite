use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_core::agents::MakeAgentsPublicRequest;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use self::caller::{AdminCaller, admin_caller};
use self::ports::{
    AgentRegistryLookup, AgentRowLookup, KillSwitchAuditWriter, KillSwitchTrigger,
    PublicGroupConfigWriter, PublicGroupStore,
};
use crate::agent_endpoints::identity_admin::error::{Detail, IdentityAdminError};
use crate::agent_endpoints::identity_admin::permission::check_agent_management_role;
use crate::audit_events::sink::AuditClock;
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::PROXY_ADMIN_ROLE;
use crate::tracing_routes::router::unauthenticated;

mod caller;
mod error;
mod kill_switch;
pub mod ports;
mod public_groups;
mod tests;

const KILL_SWITCH_PATH: &str = "/v1/agents/{agent_id}/kill_switch";
const MAKE_PUBLIC_PATH: &str = "/v1/agents/{agent_id}/make_public";
const MAKE_AGENTS_PUBLIC_PATH: &str = "/v1/agents/make_public";

// the three routes' dependencies: LiteLLM keeps them in module globals, here every one comes through the router state
#[derive(Clone)]
pub struct AgentAdminState {
    // None is prisma_client is None: kill_switch skips the row, the public-discovery routes answer db_not_connected
    pub database: Option<Arc<dyn AgentRowLookup>>,
    pub registry: Arc<dyn AgentRegistryLookup>,
    pub public_groups: Arc<dyn PublicGroupStore>,
    pub config: Arc<dyn PublicGroupConfigWriter>,
    pub kill_switch: Arc<dyn KillSwitchTrigger>,
    pub audit: Arc<dyn KillSwitchAuditWriter>,
    pub clock: Arc<dyn AuditClock>,
    pub new_id: Arc<dyn Fn() -> String + Send + Sync>,
    pub litellm_proxy_admin_name: Option<String>,
}

pub fn agent_admin_routes(state: AgentAdminState) -> Router {
    Router::new()
        .route(KILL_SWITCH_PATH, post(trigger_agent_kill_switch))
        .route(MAKE_PUBLIC_PATH, post(make_agent_public))
        .route(MAKE_AGENTS_PUBLIC_PATH, post(make_agents_public))
        .with_state(state)
}

// LiteLLM's @router.post("/v1/agents/{agent_id}/kill_switch", authenticated before the management check runs
async fn trigger_agent_kill_switch(
    State(state): State<AgentAdminState>,
    Path(agent_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let admin = match admin_caller(&context.principal) {
        Ok(admin) => admin,
        Err(error) => return error.into_response(),
    };
    // LiteLLM's _check_agent_management_permission: B4781's shared predicate over user_api_key_dict.user_role
    if let Some(refusal) = kill_switch_refusal(admin.user_role.as_deref()) {
        return refusal;
    }
    kill_switch::trigger(&state, &agent_id, &admin).await
}

// LiteLLM's @router.post("/v1/agents/{agent_id}/make_public", which requires a database before the role check
async fn make_agent_public(
    State(state): State<AgentAdminState>,
    Path(agent_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let admin = match admin_caller(&context.principal) {
        Ok(admin) => admin,
        Err(error) => return error.into_response(),
    };
    let database = match public_access(&state, &admin) {
        Ok(database) => database,
        Err(refusal) => return refusal,
    };
    public_groups::make_single(&state, database, &agent_id, &admin).await
}

// LiteLLM's @router.post("/v1/agents/make_public", the bulk replacement of the whole public list
async fn make_agents_public(
    State(state): State<AgentAdminState>,
    caller: Option<Extension<AuthContext>>,
    Json(request): Json<MakeAgentsPublicRequest>,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let admin = match admin_caller(&context.principal) {
        Ok(admin) => admin,
        Err(error) => return error.into_response(),
    };
    let database = match public_access(&state, &admin) {
        Ok(database) => database,
        Err(refusal) => return refusal,
    };
    public_groups::make_bulk(&state, database, &request.agent_ids, &admin).await
}

// both public-discovery routes answer 500 before any lookup when prisma_client is None, then check user_role inline
fn public_access<'state>(
    state: &'state AgentAdminState,
    admin: &AdminCaller,
) -> Result<&'state dyn AgentRowLookup, Response> {
    let Some(database) = state.database.as_deref() else {
        return Err(db_not_connected());
    };
    match public_groups_refusal(admin.user_role.as_deref()) {
        Some(refusal) => Err(refusal),
        None => Ok(database),
    }
}

// LiteLLM's _check_agent_management_permission answers through B4781's error, status and object detail included
fn kill_switch_refusal(role: Option<&str>) -> Option<Response> {
    match check_agent_management_role(role) {
        Ok(()) => None,
        Err(refusal) => Some(refusal.into_response()),
    }
}

// LiteLLM's inline `user_role != LitellmUserRoles.PROXY_ADMIN`, whose object detail names the caller's role
fn public_groups_refusal(role: Option<&str>) -> Option<Response> {
    if role == Some(PROXY_ADMIN_ROLE) {
        return None;
    }
    let error = format!(
        "Only proxy admins can update public model groups. Your role={}",
        role.unwrap_or("None")
    );
    let refusal = IdentityAdminError {
        status: 403,
        detail: Detail::Object { error },
    };
    Some(refusal.into_response())
}

fn db_not_connected() -> Response {
    let detail = CommonProxyErrors::DbNotConnectedError.as_str().to_owned();
    IdentityAdminError {
        status: 500,
        detail: Detail::Text(detail),
    }
    .into_response()
}
