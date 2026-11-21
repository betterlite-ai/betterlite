use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_cache::gateway_cache;
use betterlite_auth::auth_user_lookup::cache::UserCache;
use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::disk::Clock;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::decoder::TeamRecord;

use super::ports::{AdminUserReads, TeamReadError, TeamReads, UserReadError};

// betterlite-cache's MEMORY_MAX_SIZE_PER_ITEM_KIB, which is crate-private there
const CACHE_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;
const TEAMS_BY_IDS_SQL: &str = r#"SELECT row_to_json(team)::TEXT FROM "LiteLLM_TeamTable" team WHERE team.team_id = ANY($1::text[])"#;

pub struct LookupUserReads {
    pub lookup: Arc<UserLookup>,
    pub clock: Clock,
    pub management_ttl: f64,
}

impl LookupUserReads {
    // the same lookup state under another cache, so a fresh cache never outlives the call that built it
    fn lookup_with(&self, cache: Arc<dyn UserCache>) -> UserLookup {
        UserLookup {
            cache,
            store: self.lookup.store.clone(),
            cadence: Arc::clone(&self.lookup.cadence),
            db_cache_expiry: self.lookup.db_cache_expiry,
            budget_reset: Arc::clone(&self.lookup.budget_reset),
            enrollment: Arc::clone(&self.lookup.enrollment),
            tasks: self.lookup.tasks.clone(),
            defaults: self.lookup.defaults.clone(),
            redactor: Arc::clone(&self.lookup.redactor),
        }
    }
}

#[async_trait]
impl AdminUserReads for LookupUserReads {
    fn fresh_cache(&self) -> Result<Arc<dyn UserCache>, UserReadError> {
        let ordinary = LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, Arc::clone(&self.clock))
            .map_err(UserReadError::Cache)?;
        let key_objects = LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, Arc::clone(&self.clock))
            .map_err(UserReadError::Cache)?;
        let cache = gateway_cache(
            Arc::new(ordinary),
            Arc::new(key_objects),
            None,
            DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
                redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
            },
            self.management_ttl,
        );
        Ok(Arc::new(cache))
    }

    async fn user_object(
        &self,
        request: UserLookupRequest<'_>,
        cache: Arc<dyn UserCache>,
    ) -> Result<Option<LiteLLMUserTable>, UserReadError> {
        self.lookup_with(cache)
            .get_user_object(request)
            .await
            .map_err(UserReadError::Lookup)
    }
}

pub struct PostgresTeamReads {
    pub store: PostgresStore,
}

#[async_trait]
impl TeamReads for PostgresTeamReads {
    async fn teams_by_ids(&self, team_ids: &[String]) -> Result<Vec<TeamRecord>, TeamReadError> {
        let client = self
            .store
            .session()
            .await
            .map_err(TeamReadError::Database)?;
        let ids = team_ids.to_vec();
        let rows = client
            .query(TEAMS_BY_IDS_SQL, &[&ids])
            .await
            .map_err(backend)
            .map_err(TeamReadError::Database)?;
        let mut records = Vec::with_capacity(rows.len());
        for row in &rows {
            let text: String = row
                .try_get(0)
                .map_err(backend)
                .map_err(TeamReadError::Database)?;
            let record = TeamRecord::parse(text.as_bytes()).map_err(TeamReadError::Record)?;
            if let Some(found) = record {
                records.push(found);
            }
        }
        Ok(records)
    }
}
