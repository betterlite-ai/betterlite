use std::future::{Future, ready};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::agent_access_groups::reads::AccessGroupReads;
use betterlite_auth::agent_permission_handler::reads::AgentTableReads;
use betterlite_auth::auth_model_patterns::access_groups::{
    AccessGroupError, AccessGroupLookup, AccessGroupResources,
};
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::ports::{
    StoreAnswer, TeamCache, TeamCacheError, TeamStore, TeamStoreError,
};
use betterlite_auth::auth_user_lookup::cadence::CadenceClock;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use super::{CaseWorld, Failure};

impl AccessGroupReads for CaseWorld {
    fn has_database(&self) -> bool {
        self.database
    }

    fn agent_access_group_ids(&self, _agent_id: &str) -> impl Future<Output = Vec<String>> + Send {
        ready(Vec::new())
    }

    fn access_group(
        &self,
        access_group_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<AccessGroupTable, TeamLookupError>> + Send {
        let found = self.groups.get(access_group_id).cloned();
        let Some(agent_ids) = found else {
            let error = TeamLookupError::AccessGroupNotFound {
                access_group_id: access_group_id.to_owned(),
            };
            return ready(Err(error));
        };
        let row = AccessGroupTable {
            access_group_id: access_group_id.to_owned(),
            access_group_name: access_group_id.to_owned(),
            description: None,
            access_model_names: Vec::new(),
            access_mcp_server_ids: Vec::new(),
            access_agent_ids: agent_ids,
            assigned_team_ids: Vec::new(),
            assigned_key_ids: Vec::new(),
            created_at: None,
            created_by: None,
            updated_at: None,
            updated_by: None,
        };
        ready(Ok(row))
    }
}

impl AgentTableReads for CaseWorld {
    fn has_database(&self) -> bool {
        self.database
    }

    fn agent_ids_with_access_groups(
        &self,
        groups: &[String],
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Vec<String>, DbError>> + Send {
        let answer: Result<Vec<String>, DbError> = match self.fail {
            Some(Failure::Table) => Err(DbError::NoDbConnected),
            Some(Failure::Team) | None => Ok(self
                .db_agents
                .iter()
                .filter(|row| {
                    row.agent_access_groups
                        .iter()
                        .any(|group| groups.contains(group))
                })
                .map(|row| row.agent_id.clone())
                .collect()),
        };
        ready(answer)
    }
}

impl AccessGroupLookup for CaseWorld {
    fn access_group(
        &self,
        access_group_id: &str,
    ) -> Result<AccessGroupResources, AccessGroupError> {
        let found = self.groups.get(access_group_id);
        let Some(agent_ids) = found else {
            return Err(AccessGroupError::Unavailable {
                access_group_id: access_group_id.to_owned(),
            });
        };
        Ok(AccessGroupResources {
            model_names: Vec::new(),
            mcp_server_ids: Vec::new(),
            agent_ids: agent_ids.clone(),
        })
    }
}

#[async_trait]
impl TeamCache for CaseWorld {
    async fn read_team(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        Ok(None)
    }

    async fn write_team(
        &self,
        _key: &str,
        _team: &LiteLLMTeamTableCachedObj,
        _ttl: f64,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn cache_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn read_access_group(
        &self,
        _key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(None)
    }

    async fn cache_access_group(
        &self,
        _access_group_id: &str,
        _table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[async_trait]
impl TeamStore for CaseWorld {
    async fn find_team(&self, _team_id: &str, _use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        if matches!(self.fail, Some(Failure::Team)) {
            return Err(TeamStoreError::Read(Box::new(AuthReadError::Repository(
                DbError::NoDbConnected,
            ))));
        }
        Ok(self.team.clone())
    }

    async fn find_by_alias(&self, _team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        Ok(Vec::new())
    }

    async fn find_access_group(
        &self,
        _access_group_id: &str,
        _use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionCache for CaseWorld {
    async fn read(&self, _key: &str) -> PermissionAnswer {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        _ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl ObjectPermissionStore for CaseWorld {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

impl CadenceClock for CaseWorld {
    fn now(&self) -> Duration {
        Duration::from_secs(100)
    }
}
