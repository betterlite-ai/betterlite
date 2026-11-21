use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::decoder::TeamRowError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::error::InvalidationError;
use crate::auth_read::table::AuthReadError;
use crate::auth_user_lookup::ports::PortError;
use crate::resolved_key::auth::UserAPIKeyAuth;

#[derive(Debug, thiserror::Error)]
pub enum TeamCacheError {
    #[error("the auth cache failed")]
    Cache(#[source] AuthCacheError),
    #[error("the team object cache refresh failed")]
    Refresh(#[source] InvalidationError),
}

// async_get_cache/async_set_cache over B733, plus B734's _cache_team_object and _cache_access_object
#[async_trait]
pub trait TeamCache: Send + Sync {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError>;
    async fn write_team(
        &self,
        key: &str,
        team: &LiteLLMTeamTableCachedObj,
        ttl: f64,
    ) -> Result<(), TeamCacheError>;
    async fn cache_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError>;
    async fn read_access_group(
        &self,
        key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError>;
    async fn cache_access_group(
        &self,
        access_group_id: &str,
        table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError>;
}

pub struct GatewayTeamCache<Tier: AuthCacheTier> {
    pub cache: Arc<AuthCache<Tier>>,
    pub invalidation: Arc<AuthInvalidation>,
}

#[async_trait]
impl<Tier> TeamCache for GatewayTeamCache<Tier>
where
    Tier: AuthCacheTier,
{
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        self.cache
            .async_get_typed::<LiteLLMTeamTableCachedObj>(key, false)
            .await
            .map_err(|source| TeamCacheError::Cache(AuthCacheError::from(source)))
    }

    async fn write_team(
        &self,
        key: &str,
        team: &LiteLLMTeamTableCachedObj,
        ttl: f64,
    ) -> Result<(), TeamCacheError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        self.cache
            .async_set(Some(key), team, options)
            .await
            .map_err(TeamCacheError::Cache)
    }

    async fn cache_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        self.invalidation
            .cache_team_object(team_id, team)
            .await
            .map_err(TeamCacheError::Refresh)
    }

    async fn read_access_group(
        &self,
        key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        self.cache
            .async_get_typed::<AccessGroupTable>(key, false)
            .await
            .map_err(|source| TeamCacheError::Cache(AuthCacheError::from(source)))
    }

    async fn cache_access_group(
        &self,
        access_group_id: &str,
        table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        self.invalidation
            .cache_access_object(access_group_id, table)
            .await
            .map_err(TeamCacheError::Refresh)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TeamStoreError {
    #[error("the table read failed")]
    Read(#[source] Box<AuthReadError<Infallible>>),
    #[error("the stored row is not readable")]
    Row(#[source] Box<TeamRowError>),
}

pub type StoreAnswer<Answer> = Result<Answer, TeamStoreError>;

// TeamRepository and AccessGroupRepository reads through _team_table and _dictable_table
#[async_trait]
pub trait TeamStore: Send + Sync {
    async fn find_team(&self, team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>>;
    async fn find_by_alias(&self, team_alias: &str) -> StoreAnswer<Vec<JsonData>>;
    async fn find_access_group(
        &self,
        access_group_id: &str,
        use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>>;
}

// new_team's synthetic request carries only the team id and a PROXY_ADMIN principal
pub struct TeamSeed {
    pub team_id: String,
    pub principal: UserAPIKeyAuth,
}

// team_id_upsert's creation; B804 implements it with the /team/new writes and their order
#[async_trait]
pub trait TeamProvisioner: Send + Sync {
    async fn create(&self, seed: TeamSeed) -> Result<LiteLLMTeamTable, PortError>;
}
