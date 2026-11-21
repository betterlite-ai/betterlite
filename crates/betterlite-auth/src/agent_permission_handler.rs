use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use crate::agent_access_groups::access::AgentAccess;
use crate::agent_access_groups::ceiling::AgentAccessGroups;
use crate::agent_access_groups::reads::AccessGroupReads;
use crate::agent_registry::AgentRegistry;
use crate::auth_model_patterns::access_groups::AccessGroupLookup;
use crate::auth_team_lookup::lookup::TeamLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

use db_agent_ids_for_access_groups::db_agent_ids;
use error::GrantError;
use reads::AgentTableReads;

mod agent_caller_access;
mod ceiling;
// ast-grep-ignore: no-long-names
mod config_agent_ids_for_access_groups;
mod db_agent_ids_for_access_groups;
mod empty_group_denial;
pub mod error;
mod get_allowed_agents_for_key;
mod get_allowed_agents_for_team;
pub mod reads;
mod resolve_key_team_agent_access;
mod tests;
mod unified_access_group;

// LiteLLM's AgentRequestHandler over the ports the server wires once; B4780's GrantSources forwards to it
pub struct AgentGrants<Reads, Table, Unified> {
    pub(crate) registry: Arc<AgentRegistry>,
    pub(crate) ceilings: AgentAccessGroups<Reads>,
    pub(crate) teams: TeamLookup,
    pub(crate) unified: Option<Unified>,
    pub(crate) agents: Table,
}

impl<Reads, Table, Unified> AgentGrants<Reads, Table, Unified> {
    #[must_use]
    pub fn new(
        registry: Arc<AgentRegistry>,
        ceilings: AgentAccessGroups<Reads>,
        teams: TeamLookup,
        unified: Option<Unified>,
        agents: Table,
    ) -> Self {
        Self {
            registry,
            ceilings,
            teams,
            unified,
            agents,
        }
    }
}

// The type's one impl block: each method forwards to the module that ports one LiteLLM function
pub trait AgentGrantsApi {
    fn get_allowed_agents_for_key(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, GrantError>> + Send;

    fn get_allowed_agents_for_team(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, GrantError>> + Send;

    fn resolve_key_team_agent_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, GrantError>> + Send;

    fn agent_caller_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, GrantError>> + Send;

    fn agent_access_group_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<Option<BTreeSet<String>>, GrantError>> + Send;

    fn agent_ids_for_groups(
        &self,
        groups: &[String],
        check_db_only: bool,
    ) -> impl Future<Output = Result<BTreeSet<String>, GrantError>> + Send;
}

impl<Reads, Table, Unified> AgentGrantsApi for AgentGrants<Reads, Table, Unified>
where
    Reads: AccessGroupReads,
    Table: AgentTableReads,
    Unified: AccessGroupLookup,
{
    async fn get_allowed_agents_for_key(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<AgentAccess, GrantError> {
        get_allowed_agents_for_key::allowed_agents_for_key(self, auth, strict).await
    }

    async fn get_allowed_agents_for_team(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<AgentAccess, GrantError> {
        get_allowed_agents_for_team::allowed_agents_for_team(self, auth, strict).await
    }

    async fn resolve_key_team_agent_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<AgentAccess, GrantError> {
        resolve_key_team_agent_access::resolve(self, auth, strict).await
    }

    async fn agent_caller_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<AgentAccess, GrantError> {
        agent_caller_access::access(self, auth, strict).await
    }

    async fn agent_access_group_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<Option<BTreeSet<String>>, GrantError> {
        ceiling::agent_access_group_ceiling(self, auth, strict).await
    }

    async fn agent_ids_for_groups(
        &self,
        groups: &[String],
        check_db_only: bool,
    ) -> Result<BTreeSet<String>, GrantError> {
        db_agent_ids(&self.agents, groups, check_db_only).await
    }
}
