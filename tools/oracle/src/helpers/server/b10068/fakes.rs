use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError,
};
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::auth_team_lookup::ports::{TeamCache, TeamCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

// every oracle case keeps the key's team_id null, so user_belongs_to_keys_team answers before any read
pub(super) fn team_lookup() -> TeamLookup {
    TeamLookup {
        cache: Arc::new(NoTeams),
        permission_cache: Arc::new(NoTeams),
        database: None,
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

struct NoTeams;

#[async_trait]
impl TeamCache for NoTeams {
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
impl ObjectPermissionCache for NoTeams {
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
