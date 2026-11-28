use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_resource_grants::search::{
    KeySearchGrant, SearchToolFailure, TeamSearchGrant, can_user_view_search_tool,
};
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::search_tools::convert::tool::SearchTool;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_pyhost::json_values::JsonObject;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::state::TeamObjectLookup;
use crate::guardrails::admin_reads::masking::mask_sensitive_values;
use crate::key_query_plan::filters::UI_SESSION_TOKEN_TEAM_ID;
use crate::management_guards::caller::caller_has_admin_view;

#[derive(Debug, Error)]
pub(super) enum ListError {
    #[error("{0}")]
    Encode(#[source] serde_json::Error),
    #[error("{0}")]
    KeyView(#[source] serde_json::Error),
    #[error("{0}")]
    DefaultKey(#[source] AuthError),
    #[error("{0}")]
    Team(#[source] TeamLookupError),
    #[error("{0}")]
    Grant(#[source] SearchToolFailure),
}

// LiteLLM builds every key of SearchToolInfoResponse, so an absent id or timestamp is sent as null
#[derive(Debug, Serialize)]
pub(super) struct ToolView {
    pub(super) search_tool_id: Option<String>,
    pub(super) search_tool_name: String,
    pub(super) litellm_params: JsonObject,
    pub(super) search_tool_info: Option<JsonObject>,
    pub(super) created_at: Option<String>,
    pub(super) updated_at: Option<String>,
    pub(super) is_from_config: bool,
}

#[derive(Serialize)]
pub(super) struct ListBody {
    pub(super) search_tools: Vec<ToolView>,
}

// the key record keeps its fields crate-private, so the team id is read from its serde form
#[derive(Deserialize)]
struct KeyTeam {
    #[serde(default)]
    team_id: Option<String>,
}

// _get_masked_values(litellm_params, unmasked_length=4, number_of_asterisks=4); a config tool carries no id or timestamps
pub(super) fn masked_view(tool: SearchTool, from_config: bool) -> Result<ToolView, ListError> {
    let params = serde_json::to_value(&tool.litellm_params)
        .and_then(serde_json::from_value::<JsonObject>)
        .map_err(ListError::Encode)?;
    let info = match &tool.search_tool_info {
        Some(stored_info) => Some(
            serde_json::to_value(stored_info)
                .and_then(serde_json::from_value::<JsonObject>)
                .map_err(ListError::Encode)?,
        ),
        None => None,
    };
    let (search_tool_id, created_at, updated_at) = if from_config {
        (None, None, None)
    } else {
        (tool.search_tool_id, tool.created_at, tool.updated_at)
    };
    Ok(ToolView {
        search_tool_id,
        search_tool_name: tool.search_tool_name,
        litellm_params: mask_sensitive_values(&params),
        search_tool_info: info,
        created_at,
        updated_at,
        is_from_config: from_config,
    })
}

// config tools first in file order, without the unnamed ones and those a database row shadows, then every row
pub(super) fn merged_tools(
    config: Vec<SearchTool>,
    stored: Vec<SearchTool>,
) -> Result<Vec<ToolView>, ListError> {
    let stored_names: BTreeSet<&str> = stored
        .iter()
        .map(|tool| tool.search_tool_name.as_str())
        .collect();
    let mut tools = Vec::with_capacity(config.len().saturating_add(stored.len()));
    for tool in config {
        if tool.search_tool_name.is_empty() || stored_names.contains(tool.search_tool_name.as_str())
        {
            continue;
        }
        tools.push(masked_view(tool, true)?);
    }
    for tool in stored {
        tools.push(masked_view(tool, false)?);
    }
    Ok(tools)
}

// _filter_visible_search_tools: admins see all, everyone else what the key and team allowlists let them call
pub(super) async fn visible_tools(
    tools: Vec<ToolView>,
    principal: &Principal,
    teams: &dyn TeamObjectLookup,
) -> Result<Vec<ToolView>, ListError> {
    if caller_has_admin_view(principal) {
        return Ok(tools);
    }
    let key = match principal.source_key.0.as_ref() {
        Some(key) => key.clone(),
        None => UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
            .map_err(ListError::DefaultKey)?,
    };
    let team = caller_team(&key, teams).await?;
    let key_grant = KeySearchGrant { token: &key };
    let team_grant = TeamSearchGrant {
        team: team.as_ref().map(|cached| &cached.team),
    };
    let mut visible = Vec::with_capacity(tools.len());
    for tool in tools {
        if tool.search_tool_name.is_empty() {
            continue;
        }
        if can_user_view_search_tool(&tool.search_tool_name, &key_grant, &team_grant)
            .map_err(ListError::Grant)?
        {
            visible.push(tool);
        }
    }
    Ok(visible)
}

// every Admin UI session key carries the dashboard sentinel, which has no team row to look up
async fn caller_team(
    key: &UserAPIKeyAuth,
    teams: &dyn TeamObjectLookup,
) -> Result<Option<LiteLLMTeamTableCachedObj>, ListError> {
    let team_id = serde_json::to_value(key)
        .and_then(serde_json::from_value::<KeyTeam>)
        .map_err(ListError::KeyView)?
        .team_id;
    match team_id.as_deref() {
        Some(id) if !id.is_empty() && id != UI_SESSION_TOKEN_TEAM_ID => {
            let team = teams.team_object(id, key).await.map_err(ListError::Team)?;
            Ok(Some(team))
        }
        Some(_) | None => Ok(None),
    }
}
