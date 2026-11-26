use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer as OrgAnswer};
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use super::Fixture;

#[async_trait]
impl TeamCache for Fixture {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        Ok(self
            .teams
            .get(key.strip_prefix("team_id:").unwrap_or(key))
            .cloned())
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
        key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(self
            .groups
            .get(key.strip_prefix("access_group_id:").unwrap_or(key))
            .cloned())
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
impl TeamStore for Fixture {
    async fn find_team(&self, _team_id: &str, _use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
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
impl ObjectPermissionCache for Fixture {
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
        60.0
    }
}

#[async_trait]
impl ObjectPermissionStore for Fixture {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

#[async_trait]
impl OrgCache for Fixture {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        Ok(None)
    }

    async fn write_org(
        &self,
        _key: &str,
        _org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        Ok(self
            .projects
            .get(key.strip_prefix("project_id:").unwrap_or(key))
            .cloned())
    }

    async fn write_project(
        &self,
        _key: &str,
        _project: &CachedProject,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl OrgStore for Fixture {
    async fn find_org(&self, _org_id: &str, _include_budget: bool) -> OrgAnswer<Option<String>> {
        Ok(None)
    }

    async fn find_by_alias(&self, _org_alias: &str) -> OrgAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> OrgAnswer<Option<String>> {
        Ok(None)
    }
}
