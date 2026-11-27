use async_trait::async_trait;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use crate::agent_access_groups::ceiling::AgentAccessGroupCeiling;
use crate::agent_access_groups::error::AccessGroupsError;
use crate::agent_caller::error::AgentCallerError;
use crate::agent_policy::AgentResponse;
use crate::resolved_key::auth::UserAPIKeyAuth;

// the six agent collaborators the MIT checks read; B8646 implements this over the proxy's registry
#[async_trait]
pub trait AgentAccessSource: Send + Sync {
    // managed_agent_policy: the managed record the key already carries, never a registry lookup
    fn managed_policy(&self, auth: Option<&UserAPIKeyAuth>) -> Option<AgentResponse>;

    // resolve_managed_agent_ceilings: the ceilings of the managed record's own access group ids
    async fn managed_ceilings(
        &self,
        policy: &AgentResponse,
    ) -> Result<Vec<AgentAccessGroupCeiling>, AccessGroupsError>;

    // resolve_agent_access_group_ceiling(agent_id): None when the agent lists no group
    async fn unmanaged_ceiling(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentAccessGroupCeiling>, AccessGroupsError>;

    // agent_caller_auth: the stand-in record for the invoking user and team
    async fn caller_auth(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Option<UserAPIKeyAuth>, AgentCallerError>;

    // load_agent_caller_team: a team that cannot be read is an error, never "no team"
    async fn caller_team(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, AgentCallerError>;

    // load_agent_caller_user: a user id with no row narrows nothing
    async fn caller_user(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMUserTable>, AgentCallerError>;
}
