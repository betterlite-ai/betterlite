use super::AgentGrants;
use super::error::GrantError;
use super::get_allowed_agents_for_team;
use super::reads::AgentTableReads;
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_caller::caller_auth::agent_caller_auth;
use crate::auth_model_patterns::access_groups::AccessGroupLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// agent_caller_access: a key without a forwarded caller has no cap; a caller reaches its own team's agents
pub(crate) async fn access<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    auth: Option<&UserAPIKeyAuth>,
    strict: bool,
) -> Result<AgentAccess, GrantError>
where
    Table: AgentTableReads,
    Unified: AccessGroupLookup,
{
    let Some(auth) = auth else {
        return Ok(AgentAccess::Unrestricted);
    };
    let Some(caller) = agent_caller_auth(auth).map_err(GrantError::from)? else {
        return Ok(AgentAccess::Unrestricted);
    };
    get_allowed_agents_for_team::allowed_agents_for_team(grants, Some(&caller), strict).await
}
