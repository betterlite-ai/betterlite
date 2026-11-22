use std::collections::BTreeSet;

use tracing::warn;

use super::AgentGrants;
use super::db_agent_ids_for_access_groups::db_agent_ids;
use super::error::GrantError;
use super::reads::AgentTableReads;
use super::unified_access_group;
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_access_groups::reads::AccessGroupReads;
use crate::auth_model_patterns::access_groups::AccessGroupLookup;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamRequest, TeamSource};
use crate::auth_team_lookup::team::get_team_object;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// LiteLLM's UI_TEAM_ID: the dashboard team never has agent grants, so a failed read there logs nothing
const UI_TEAM_ID: &str = "litellm-dashboard";

// _get_allowed_agents_for_team: the team object fetched once, then the same three sources as the key
pub(crate) async fn allowed_agents_for_team<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    auth: Option<&UserAPIKeyAuth>,
    strict: bool,
) -> Result<AgentAccess, GrantError>
where
    Reads: AccessGroupReads,
    Table: AgentTableReads,
    Unified: AccessGroupLookup,
{
    let Some(key) = auth else {
        return Ok(AgentAccess::Unrestricted);
    };
    let Some(team_id) = key.view.token.team_id.as_deref() else {
        return Ok(AgentAccess::Unrestricted);
    };
    match declared_team_access(grants, team_id, strict).await {
        Ok(access) => Ok(access),
        Err(error) if strict => Err(GrantError::PolicyUnavailable(Box::new(error))),
        Err(error) => {
            if key.view.token.team_id.as_deref() != Some(UI_TEAM_ID) {
                warn!("Failed to get allowed agents for team: {error}");
            }
            Ok(AgentAccess::Unrestricted)
        }
    }
}

async fn declared_team_access<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    team_id: &str,
    strict: bool,
) -> Result<AgentAccess, GrantError>
where
    Reads: AccessGroupReads,
    Table: AgentTableReads,
    Unified: AccessGroupLookup,
{
    let request = TeamRequest {
        team_id,
        source: if strict {
            TeamSource::Writer
        } else {
            TeamSource::CacheThenDatabase
        },
        team_id_upsert: false,
    };
    let team = match get_team_object(&grants.teams, request).await {
        Ok(team) => team,
        Err(TeamLookupError::NoDatabase) => return Ok(deny_when_strict(strict)),
        Err(error) => return Err(GrantError::from(error)),
    };
    let fields = &team.team.fields;
    let permission = fields.object_permission.as_ref();
    let direct = permission
        .and_then(|declared| declared.agents.0.as_deref())
        .unwrap_or(&[]);
    let declared = permission
        .and_then(|declared| declared.agent_access_groups.0.as_deref())
        .unwrap_or(&[]);
    let unified = fields.base.access_group_ids.as_deref().unwrap_or(&[]);
    if direct.is_empty() && declared.is_empty() && unified.is_empty() {
        return Ok(AgentAccess::Unrestricted);
    }
    let mut ids: BTreeSet<String> = direct.iter().cloned().collect();
    if !declared.is_empty() {
        ids.extend(db_agent_ids(&grants.agents, declared, strict).await?);
    }
    if !unified.is_empty() {
        ids.extend(unified_access_group::agent_ids(
            grants.unified.as_ref(),
            unified,
            strict,
        )?);
    }
    Ok(AgentAccess::Restricted(ids))
}

const fn deny_when_strict(strict: bool) -> AgentAccess {
    if strict {
        AgentAccess::Restricted(BTreeSet::new())
    } else {
        AgentAccess::Unrestricted
    }
}
