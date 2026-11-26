#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::team_budget_admission::sources::MembershipSource;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use super::{EntityModelRule, rule};

struct Rows {
    live: bool,
    read: Arc<AtomicBool>,
}

#[async_trait]
impl MembershipSource for Rows {
    async fn team_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        assert_eq!(user_id, "user-1");
        assert_eq!(team_id, "team-1");
        self.read.store(true, Ordering::Release);
        let row = serde_json::from_str(r#"{"user_id":"user-1","team_id":"team-1","budget_id":"budget-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#).unwrap();
        Ok(Some(row))
    }
}

#[async_trait]
impl TeamCache for Rows {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        assert_eq!(key, "team_id:team-1");
        let row = serde_json::from_str(r#"{"team_id":"team-1","models":["claude"]}"#).unwrap();
        Ok(self.live.then_some(row))
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
        _id: &str,
        _group: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[async_trait]
impl TeamStore for Rows {
    async fn find_team(&self, _team_id: &str, _writer: bool) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }
    async fn find_by_alias(&self, _alias: &str) -> StoreAnswer<Vec<JsonData>> {
        Ok(Vec::new())
    }
    async fn find_access_group(&self, _id: &str, _writer: bool) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionCache for Rows {
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
impl ObjectPermissionStore for Rows {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

fn lookup(rows: &Arc<Rows>) -> TeamLookup {
    let cache: Arc<dyn TeamCache> = Arc::<Rows>::clone(rows);
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::<Rows>::clone(rows);
    let store: Arc<dyn TeamStore> = Arc::<Rows>::clone(rows);
    let reader: Arc<dyn ObjectPermissionStore> = Arc::<Rows>::clone(rows);
    let writer: Arc<dyn ObjectPermissionStore> = Arc::<Rows>::clone(rows);
    TeamLookup {
        cache,
        permission_cache,
        database: Some(TeamDatabase {
            store,
            permission_reader: reader,
            permission_writer: writer,
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(16, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

pub(super) fn rule_with_member(live: bool) -> (Arc<EntityModelRule>, Arc<AtomicBool>) {
    let read = Arc::new(AtomicBool::new(false));
    let rows = Arc::new(Rows {
        live,
        read: Arc::clone(&read),
    });
    let mut grants =
        rule(r#"{"budget_id":"b","created_by":"a","updated_by":"b","models":["claude"]}"#);
    let fields = serde_json::from_str(
        r#"{"token":"hash-1","team_id":"team-1","user_id":"user-1","team_models":["claude"]}"#,
    )
    .unwrap();
    let access = Arc::get_mut(&mut grants).unwrap();
    access.key = Some(UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap());
    access.teams = Some(Arc::new(lookup(&rows)));
    Arc::get_mut(&mut access.member_checks).unwrap().memberships = Some(rows);
    (grants, read)
}
