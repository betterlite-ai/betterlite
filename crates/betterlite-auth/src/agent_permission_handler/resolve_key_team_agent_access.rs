use super::AgentGrants;
use super::error::GrantError;
use super::get_allowed_agents_for_key;
use super::get_allowed_agents_for_team;
use super::reads::AgentTableReads;
use crate::agent_access_groups::access::{AgentAccess, intersect_agent_access};
use crate::auth_model_patterns::access_groups::AccessGroupLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// resolve_key_team_agent_access: LiteLLM's try/except is already inside each side (PolicyUnavailable when strict, Unrestricted otherwise)
pub(crate) async fn resolve<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    auth: Option<&UserAPIKeyAuth>,
    strict: bool,
) -> Result<AgentAccess, GrantError>
where
    Table: AgentTableReads,
    Unified: AccessGroupLookup,
{
    let key_access =
        get_allowed_agents_for_key::allowed_agents_for_key(grants, auth, strict).await?;
    let team_access =
        get_allowed_agents_for_team::allowed_agents_for_team(grants, auth, strict).await?;
    Ok(intersect_agent_access(
        &grants.registry,
        &key_access,
        &team_access,
    ))
}
