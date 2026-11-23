use std::collections::BTreeSet;

use axum::Json;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::agents::AgentMakePublicResponse;

use super::AgentAdminState;
use super::caller::AdminCaller;
use super::error::{missing_agent, unexpected_failure};
use super::ports::{AgentRowLookup, KnownAgent};
use crate::agent_endpoints::identity_admin::error::{Detail, IdentityAdminError};

const SUCCESS_MESSAGE: &str = "Successfully updated public agent groups";

// make_agent_public: append the found agent's id, save the config, then publish the list public discovery reads
pub(super) async fn make_single(
    state: &AgentAdminState,
    database: &dyn AgentRowLookup,
    agent_id: &str,
    caller: &AdminCaller,
) -> Response {
    let agent = match known_agent(state, database, agent_id).await {
        Ok(agent) => agent,
        Err(error) => return error.into_response(),
    };
    let current = state.public_groups.groups().unwrap_or_default();
    let listed: BTreeSet<String> = current.iter().cloned().collect();
    if !state
        .registry
        .ids_for_agent(&agent.agent_id)
        .is_disjoint(&listed)
    {
        let detail = format!(
            "Agent with name {} already in public agent groups",
            agent.agent_name
        );
        let error = IdentityAdminError {
            status: 400,
            detail: Detail::Text(detail),
        };
        return error.into_response();
    }
    let mut updated = current;
    updated.push(agent.agent_id);
    if let Err(error) = state.config.save_public_groups(&updated).await {
        return unexpected_failure(error).into_response();
    }
    state.public_groups.set_groups(updated.clone());
    Json(make_public_response(updated, caller)).into_response()
}

// make_agents_public: check every supplied id exists, then replace the list in memory before saving the config
pub(super) async fn make_bulk(
    state: &AgentAdminState,
    database: &dyn AgentRowLookup,
    agent_ids: &[String],
    caller: &AdminCaller,
) -> Response {
    if let Some(error) = missing_listed_agent(state, database, agent_ids).await {
        return error.into_response();
    }
    let groups = agent_ids.to_vec();
    state.public_groups.set_groups(groups.clone());
    if let Err(error) = state.config.save_public_groups(&groups).await {
        return unexpected_failure(error).into_response();
    }
    Json(make_public_response(groups, caller)).into_response()
}

// AGENT_REGISTRY.get_agent_by_id first, then the row the agents table holds for an id the registry does not know
async fn known_agent(
    state: &AgentAdminState,
    database: &dyn AgentRowLookup,
    agent_id: &str,
) -> Result<KnownAgent, IdentityAdminError> {
    if let Some(agent) = state.registry.agent_by_id(agent_id) {
        return Ok(agent);
    }
    database
        .agent_row(agent_id)
        .await
        .map_err(unexpected_failure)?
        .ok_or_else(|| missing_agent(agent_id))
}

async fn missing_listed_agent(
    state: &AgentAdminState,
    database: &dyn AgentRowLookup,
    agent_ids: &[String],
) -> Option<IdentityAdminError> {
    for agent_id in agent_ids {
        if let Err(error) = known_agent(state, database, agent_id).await {
            return Some(error);
        }
    }
    None
}

fn make_public_response(groups: Vec<String>, caller: &AdminCaller) -> AgentMakePublicResponse {
    AgentMakePublicResponse {
        message: String::from(SUCCESS_MESSAGE),
        public_agent_groups: groups,
        // LiteLLM declares updated_by as a str, so a caller with no user_id gets an empty one instead of a 500
        updated_by: caller.user_id.clone().unwrap_or_default(),
    }
}
