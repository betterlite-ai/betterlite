use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::{AgentKillSwitchConfig, AgentKillSwitchResult};
use serde::Serialize;

use super::AgentAdminState;
use super::caller::AdminCaller;
use super::error::{missing_agent, unexpected_failure};
use crate::agent_credentials::kill_switch::parse_agent_kill_switch;
use crate::agent_endpoints::identity_admin::error::{Detail, IdentityAdminError};
use crate::audit_events::identity::CallerIdentity;
use crate::kill_switch_management::audit::{KillSwitchAuditContext, build_kill_switch_audit_log};

// FastAPI renders HTTPException(detail=result.model_dump()) as {"detail": ...}, so the 502 carries the same result under detail
#[derive(Serialize)]
struct FailureBody {
    detail: AgentKillSwitchResult,
}

// what the kill switch lookup found: no such agent answers 404, an agent with no switch answers 400
enum Resolution {
    Missing,
    NoSwitch,
    Switch(Box<AgentKillSwitchConfig>),
}

// the route body: _resolve_agent_kill_switch, the webhook attempt, the audit row, then 200 or 502 with the same result
pub(super) async fn trigger(
    state: &AgentAdminState,
    agent_id: &str,
    caller: &AdminCaller,
) -> Response {
    let resolved = match resolve(state, agent_id).await {
        Ok(resolved) => resolved,
        Err(error) => return error.into_response(),
    };
    let configured = match resolved {
        Resolution::Missing => return missing_agent(agent_id).into_response(),
        Resolution::NoSwitch => {
            let detail = format!("Agent with ID {agent_id} has no kill_switch configured");
            let error = IdentityAdminError {
                status: 400,
                detail: Detail::Text(detail),
            };
            return error.into_response();
        }
        Resolution::Switch(configured) => configured,
    };
    let result = state.kill_switch.fire(agent_id, &configured).await;
    record_attempt(state, &result, caller).await;
    if result.succeeded() {
        return Json(result).into_response();
    }
    let body = FailureBody { detail: result };
    (StatusCode::BAD_GATEWAY, Json(body)).into_response()
}

// the DB row wins over this replica's registry; a config.yaml agent has no row and falls back to the registry
async fn resolve(
    state: &AgentAdminState,
    agent_id: &str,
) -> Result<Resolution, IdentityAdminError> {
    if let Some(database) = state.database.as_deref() {
        let found = database
            .agent_row(agent_id)
            .await
            .map_err(unexpected_failure)?;
        if let Some(stored) = found {
            return Ok(resolution(stored.kill_switch.as_ref()));
        }
    }
    let Some(agent) = state.registry.agent_by_id(agent_id) else {
        return Ok(Resolution::Missing);
    };
    Ok(resolution(agent.kill_switch.as_ref()))
}

fn resolution(stored: Option<&AgentJson>) -> Resolution {
    match parse_agent_kill_switch(stored) {
        Some(config) => Resolution::Switch(Box::new(config)),
        None => Resolution::NoSwitch,
    }
}

// the audit row comes after the attempt, so a failed webhook leaves one too, and its write never changes the answer
async fn record_attempt(
    state: &AgentAdminState,
    result: &AgentKillSwitchResult,
    caller: &AdminCaller,
) {
    // this audit passes no litellm-changed-by header, so the metadata opt-in get_audit_log_changed_by reads is never consulted
    let identity = CallerIdentity {
        user_id: caller.user_id.clone(),
        api_key: caller.api_key.clone(),
        metadata: None,
        team_metadata: None,
    };
    let context = KillSwitchAuditContext {
        caller: &identity,
        litellm_proxy_admin_name: state.litellm_proxy_admin_name.as_deref(),
        clock: state.clock.as_ref(),
        id: (state.new_id)(),
    };
    let Ok(record) = build_kill_switch_audit_log(result, context) else {
        tracing::error!("Failed building the kill switch audit log");
        return;
    };
    state.audit.write(record).await;
}
