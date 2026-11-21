use std::collections::BTreeSet;

use betterlite_db::resource_rows::access_group::AccessGroupTable;
use tracing::warn;

use super::error::AccessGroupsError;
use super::reads::AccessGroupReads;

mod tests;

// An empty set denies that resource kind
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentAccessGroupCeiling {
    pub access_group_ids: Vec<String>,
    pub models: BTreeSet<String>,
    pub mcp_server_ids: BTreeSet<String>,
    pub agent_ids: BTreeSet<String>,
}

pub struct AgentAccessGroups<Reads> {
    pub reads: Reads,
}

impl<Reads: AccessGroupReads> AgentAccessGroups<Reads> {
    // None only when the agent lists no group, so nothing is capped
    pub async fn resolve_agent_access_group_ceiling(
        &self,
        agent_id: &str,
        check_db_only: bool,
    ) -> Result<Option<AgentAccessGroupCeiling>, AccessGroupsError> {
        let access_group_ids = self.reads.agent_access_group_ids(agent_id).await;
        if access_group_ids.is_empty() {
            return Ok(None);
        }
        self.union_groups(access_group_ids, check_db_only)
            .await
            .map(Some)
    }

    pub async fn agent_access_group_server_ids(
        &self,
        agent_id: &str,
    ) -> Result<Option<Vec<String>>, AccessGroupsError> {
        Ok(self
            .resolve_agent_access_group_ceiling(agent_id, false)
            .await?
            .map(|ceiling| ceiling.mcp_server_ids.into_iter().collect()))
    }

    // ids from the managed record the caller holds: a stale registry copy cannot widen it
    pub async fn resolve_managed_agent_ceilings(
        &self,
        access_group_ids: &[String],
    ) -> Result<Vec<AgentAccessGroupCeiling>, AccessGroupsError> {
        if access_group_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ceiling = self.union_groups(access_group_ids.to_vec(), true).await?;
        Ok(vec![ceiling])
    }

    async fn union_groups(
        &self,
        access_group_ids: Vec<String>,
        check_db_only: bool,
    ) -> Result<AgentAccessGroupCeiling, AccessGroupsError> {
        let mut models = BTreeSet::new();
        let mut mcp_server_ids = BTreeSet::new();
        let mut agent_ids = BTreeSet::new();
        for access_group_id in &access_group_ids {
            // a group that did not load keeps its id listed and adds nothing, so it denies
            let Some(group) = self.load_group(access_group_id, check_db_only).await? else {
                continue;
            };
            models.extend(group.access_model_names);
            mcp_server_ids.extend(group.access_mcp_server_ids);
            agent_ids.extend(group.access_agent_ids);
        }
        Ok(AgentAccessGroupCeiling {
            access_group_ids,
            models,
            mcp_server_ids,
            agent_ids,
        })
    }

    async fn load_group(
        &self,
        access_group_id: &str,
        check_db_only: bool,
    ) -> Result<Option<AccessGroupTable>, AccessGroupsError> {
        if !self.reads.has_database() {
            warn!(
                access_group_id,
                "Agent access group {access_group_id} cannot be loaded without a DB"
            );
            return Ok(None);
        }
        match self
            .reads
            .access_group(access_group_id, check_db_only)
            .await
        {
            Ok(group) => Ok(Some(group)),
            // LiteLLM drops only an HTTPException, and only outside the writer path
            Err(error) if !check_db_only && error.http().is_some() => {
                warn!(
                    access_group_id,
                    "Agent access group {access_group_id} could not be loaded, treating it as empty: {error}"
                );
                Ok(None)
            }
            Err(source) => Err(AccessGroupsError::Read {
                access_group_id: access_group_id.to_owned(),
                source: Box::new(source),
            }),
        }
    }
}
